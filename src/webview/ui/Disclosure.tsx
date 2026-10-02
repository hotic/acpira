import { createContext, useContext, useId, useState, type ReactNode } from 'react';
import { cn } from './cn';
import { Collapsible } from './Collapsible';
import { Row, type RowProps } from './Row';
import { ConnectedRail } from './ConnectedRail';

// Lets an enclosing fold learn that a nested row was toggled by hand, without owning its state.
export const DisclosureObserverContext = createContext<((open: boolean) => void) | undefined>(undefined);

// An expandable row: the summary is just a Row (button), the body expands with a height animation, indented to align with the lead slot.
// The parent container is a flex column so the button spans the full row
export interface DisclosureProps extends Omit<RowProps, 'as' | 'interactive' | 'onToggle'> {
  open?: boolean;
  defaultOpen?: boolean;
  body: ReactNode;
  indent?: boolean;
  rail?: 'body' | 'rows' | false;
  onToggle?: (open: boolean) => void;
  /** Extra classes on the body's padding box, e.g. a density-aware top gap */
  bodyClassName?: string;
  /** Keep a file opener beside the fold trigger rather than nesting buttons. */
  independentAction?: boolean;
}

export function DisclosureRow({ independentAction, ...row }: RowProps & { independentAction?: boolean }) {
  const labelId = useId();
  if (!independentAction) return <Collapsible.Trigger render={<Row as="button" interactive {...row} />} />;
  // The full-row trigger sits behind the label. Only explicit actions receive pointer events above it.
  return <div className="group/disclosure-row relative">
    <Collapsible.Trigger aria-labelledby={labelId} className="absolute -inset-x-hit inset-y-0 cursor-pointer rounded-md group-hover/disclosure-row:bg-hover focus-visible:bg-hover" />
    <Row {...row} id={labelId} className={cn('relative pointer-events-none', row.className)} />
  </div>;
}

// Rule: a body that is indented past the lead slot (i.e. not full width) gets a rail down that slot; full-width bodies (cards, lists) get none
export function Disclosure({ body, open: controlled, defaultOpen = false, indent = true, rail = indent ? 'body' : false, onToggle, className, bodyClassName, independentAction, ...row }: DisclosureProps) {
  const [inner, setInner] = useState(defaultOpen);
  const observe = useContext(DisclosureObserverContext);
  const open = controlled ?? inner;
  const toggle = (next: boolean) => { setInner(next); onToggle?.(next); observe?.(next); };
  return (
    <Collapsible.Root open={open} onOpenChange={toggle} render={<ConnectedRail enabled={!!rail && open && row.lead !== undefined}
      endAtLastRow={rail === 'rows'}
      selector={rail === 'body' ? independentAction ? ':scope > div > .action-row > .row-lead' : ':scope > button > .row-lead' : undefined}
      className={cn('group flex min-w-0 flex-col', className)} data-open={open || undefined} />}>
      <DisclosureRow {...row} independentAction={independentAction} />
      {/* Nested rows extend their hit area beyond the text column; reserve it inside the clip so its edges cannot cut off row corners. */}
      <Collapsible.Panel className="-mx-hit [&>div]:px-hit">
        <div className={cn('pt-1', !rail && 'pb-1.5', indent && row.lead !== undefined && 'pl-indent', bodyClassName)}>{body}</div>
      </Collapsible.Panel>
    </Collapsible.Root>
  );
}
