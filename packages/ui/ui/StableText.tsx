import type { ReactNode } from 'react';
import { cn } from './cn';

export interface StableItem { key: string; text: ReactNode }

// Text that changes with a pick but keeps the box of its largest variant. Every variant sits in one grid cell and only
// the current one is visible, so switching never moves the layout around it: a select stays as wide as its longest
// label, a setting row as tall as its longest description. The hidden copies wrap at the live width, so the reserved
// size follows the container instead of a measured snapshot; `invisible` and `aria-hidden` keep them out of find,
// selection and the accessibility tree
export function StableText({ items, current, className, itemClassName }: { items: readonly StableItem[]; current: string; className?: string; itemClassName?: string }) {
  return (
    <span className={cn('grid grid-cols-[minmax(0,1fr)]', className)}>
      {items.map(i => (
        <span key={i.key} aria-hidden={i.key !== current || undefined} className={cn('col-start-1 row-start-1', itemClassName, i.key !== current && 'invisible')}>{i.text}</span>
      ))}
    </span>
  );
}
