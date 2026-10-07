import { createContext, useContext, useEffect, useRef, useState, type ComponentProps, type ReactNode } from 'react';
import { Collapsible as Base } from '@base-ui/react/collapsible';
import { cn, cnState } from './cn';
import { cssTimeVar } from './cssTime';

// Settled transcript content: a panel under `true` mounts its body only while open. Finished turns keep almost all of
// their DOM inside closed folds; mounting it made every session switch lay out the whole history, and keeping a body
// once opened let an expand / collapse pass over a long session pin every fold's DOM for the page's lifetime.
// A closed body is released after the close transition; under `false` (a live turn) bodies stay mounted so a closed
// thought still streams in step.
export const LazyPanelContext = createContext(false);

// Fallback for --dur-close when the variable cannot be read, plus a frame or two for the transition to finish
const DEFAULT_CLOSE_MS = 320;
const RELEASE_SLACK_MS = 80;

const PANEL = 'grid min-w-0 grid-cols-[minmax(0,1fr)] transition-[grid-template-rows,opacity] duration-(--dur-open) ease-out data-[open]:grid-rows-[1fr] data-[open]:opacity-100 data-[closed]:grid-rows-[0fr] data-[closed]:opacity-0 data-[closed]:duration-(--dur-close) data-[closed]:ease-(--ease-close)';

function Panel({ children, className, ...props }: ComponentProps<typeof Base.Panel>) {
  return <Base.Panel {...props} keepMounted hidden={false}
    render={(attributes, state) => <PanelFrame {...attributes} open={state.open} />}
    className={cnState(PANEL, className)}>
    {children}
  </Base.Panel>;
}

// A further part of a root's panel, for a body broken by content that stays visible while it is closed (a steered
// prompt between process segments): each part closes on the panel's own curve, so the visible content slides with
// them instead of being drawn twice. Base UI tracks one panel per root, so a section follows `open` directly.
function Section({ open, className, children }: { open: boolean; className?: string; children: ReactNode }) {
  return <PanelFrame open={open} className={cn(PANEL, className)} data-open={open ? '' : undefined} data-closed={open ? undefined : ''}>{children}</PanelFrame>;
}

function PanelFrame({ open, children, ...attributes }: ComponentProps<'div'> & { open: boolean }) {
  const lazy = useContext(LazyPanelContext);
  const frame = useRef<HTMLDivElement>(null);
  const [kept, setKept] = useState(open || !lazy);
  // Mount synchronously on open (no empty first frame); the release below waits for the close animation.
  if ((open || !lazy) && !kept) setKept(true);
  useEffect(() => {
    if (open || !lazy || !kept) return;
    const el = frame.current;
    // The shipped stylesheet is minified to `.32s`: read the token with its unit (cssTime.ts)
    const close = el ? cssTimeVar(el, '--dur-close', DEFAULT_CLOSE_MS) : DEFAULT_CLOSE_MS;
    const timer = setTimeout(() => setKept(false), close + RELEASE_SLACK_MS);
    return () => clearTimeout(timer);
  }, [open, lazy, kept]);
  // Inert replaces hidden so nested rails can keep measuring their mounted DOM.
  // The ref sits on the inner clip: `attributes` carries Base UI's own ref for the panel element.
  return <div {...attributes} inert={!open}>
    <div ref={frame} className="min-h-0 min-w-0 overflow-hidden">{(kept || open || !lazy) && children}</div>
  </div>;
}

export const Collapsible = { Root: Base.Root, Trigger: Base.Trigger, Panel, Section };
