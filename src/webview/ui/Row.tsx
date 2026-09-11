import { createContext, useContext, useState, type HTMLAttributes, type ReactNode, type Ref } from 'react';
import { cn } from './cn';
import { cva } from 'class-variance-authority';
import { Shimmer } from './Shimmer';

// Scope entrance effects to live transcript rows; menus and restored history stay still.
export const RowEntranceContext = createContext(false);

// The one shared "row": thought / plan / tool / status / session items all grow on this row.
// Row height --row; lead slot --lead (icon 14 or Orb 20 centered); label area gap --gap; trailing meta right-aligned.
export interface RowProps extends Omit<HTMLAttributes<HTMLElement>, 'children'> {
  ref?: Ref<HTMLElement>;
  lead?: ReactNode;
  trailing?: ReactNode;
  children?: ReactNode;
  interactive?: boolean;
  as?: 'div' | 'button';
  className?: string;
  dense?: boolean;
  tone?: 'action';
}

const rowVariants = cva('flex items-center gap-gap text-2 text-fg-2 select-none list-none text-left', {
  variants: {
    tone: { action: 'action-row' },
    dense: { true: 'min-h-row-dense', false: 'min-h-row' },
    interactive: { true: '-mx-hit px-hit cursor-pointer rounded-md hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 transition-colors' },
    enter: { true: 'process-row-enter' },
  },
});

export function Row({ lead, trailing, children, interactive, as = 'div', className, dense, tone, ref, ...rest }: RowProps) {
  const Tag = as;
  const live = useContext(RowEntranceContext);
  const [enter] = useState(live);
  return (
    <Tag
      ref={ref as Ref<HTMLButtonElement & HTMLDivElement>}
      {...(as === 'button' ? { type: 'button' } : {})}
      className={cn(
        rowVariants({ dense: !!dense, interactive, enter, tone }),
        className,
      )}
      {...rest}
    >
      <span className={cn('row-lead size-lead shrink-0 items-center justify-center text-fg-3', lead === undefined ? 'row-lead-empty hidden' : 'flex')}>{lead}</span>
      <span className="row-content flex min-w-0 flex-1 items-baseline gap-2">{children}</span>
      {trailing !== undefined && <span className="row-trailing ml-auto flex shrink-0 items-center gap-2 text-3 text-fg-3 tabular-nums">{trailing}</span>}
    </Tag>
  );
}

// Keep short labels intact; the adjacent target gives up space and truncates first. `shimmer` marks the running verb.
export function RowLabel({ children, className, shimmer }: { children: ReactNode; className?: string; shimmer?: boolean }) {
  return <Shimmer active={!!shimmer} className={cn('shrink-0 whitespace-nowrap', className)}>{children}</Shimmer>;
}

// Target text within a row (file name / command), one step brighter than the verb
export function RowTarget({ children, mono, className }: { children: ReactNode; mono?: boolean; className?: string }) {
  return <span className={cn('row-target truncate text-fg-1/85', mono && 'font-mono text-mono', className)}>{children}</span>;
}
