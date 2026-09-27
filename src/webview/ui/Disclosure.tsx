import { createContext, useContext, useState, type ReactNode } from 'react';
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
}

// Rule: a body that is indented past the lead slot (i.e. not full width) gets a rail down that slot; full-width bodies (cards, lists) get none
export function Disclosure({ body, open: controlled, defaultOpen = false, indent = true, rail = indent ? 'body' : false, onToggle, className, bodyClassName, ...row }: DisclosureProps) {
  const [inner, setInner] = useState(defaultOpen);
  const observe = useContext(DisclosureObserverContext);
  const open = controlled ?? inner;
  const toggle = (next: boolean) => { setInner(next); onToggle?.(next); observe?.(next); };
  return (
    <Collapsible.Root open={open} onOpenChange={toggle} render={<ConnectedRail enabled={!!rail && open && row.lead !== undefined}
      endAtLastRow={rail === 'rows'}
      selector={rail === 'body' ? ':scope > button > .row-lead' : undefined}
      className={cn('group flex min-w-0 flex-col', className)} data-open={open || undefined} />}>
      <Collapsible.Trigger render={<Row as="button" interactive {...row} />} />
      {/* Nested rows extend their hit area beyond the text column; reserve it inside the clip so its edges cannot cut off row corners. */}
      <Collapsible.Panel className="-mx-hit [&>div]:px-hit">
        <div className={cn('pt-1', !rail && 'pb-1.5', indent && row.lead !== undefined && 'pl-indent', bodyClassName)}>{body}</div>
      </Collapsible.Panel>
    </Collapsible.Root>
  );
}
