import { createContext, useContext, useRef, type ComponentProps } from 'react';
import { Collapsible as Base } from '@base-ui/react/collapsible';
import { cn, cnState } from './cn';

// Settled transcript content: a panel under `true` that has never been open does not mount its body. Finished turns keep
// almost all of their DOM inside closed folds, and mounting it made every session switch lay out the whole history.
// A body mounted once (opened, or rendered while the value was false) stays mounted.
export const LazyPanelContext = createContext(false);

function Panel({ children, className, ...props }: ComponentProps<typeof Base.Panel>) {
  return <Base.Panel {...props} keepMounted hidden={false}
    render={(attributes, state) => <PanelFrame {...attributes} open={state.open} />}
    className={cnState(cn('grid min-w-0 grid-cols-[minmax(0,1fr)] transition-[grid-template-rows,opacity] duration-(--dur-open) ease-out data-[open]:grid-rows-[1fr] data-[open]:opacity-100 data-[closed]:grid-rows-[0fr] data-[closed]:opacity-0'), className)}>
    {children}
  </Base.Panel>;
}

function PanelFrame({ open, children, ...attributes }: ComponentProps<'div'> & { open: boolean }) {
  const lazy = useContext(LazyPanelContext);
  const seen = useRef(false);
  if (open || !lazy) seen.current = true;
  // Inert replaces hidden so nested rails can keep measuring their mounted DOM.
  return <div {...attributes} inert={!open}>
    <div className="min-h-0 min-w-0 overflow-hidden">{seen.current && children}</div>
  </div>;
}

export const Collapsible = { Root: Base.Root, Trigger: Base.Trigger, Panel };
