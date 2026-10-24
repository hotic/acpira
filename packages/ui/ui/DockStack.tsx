import type { HTMLAttributes, Ref } from 'react';
import { cn } from './cn';

// The composer dock is one vertical rhythm. Children keep their own inner
// padding while this stack owns the distance between sibling surfaces.
export function DockStack({ className, ref, ...rest }: HTMLAttributes<HTMLDivElement> & { ref?: Ref<HTMLDivElement> }) {
  return <div ref={ref} className={cn('flex shrink-0 flex-col gap-(--dock-gap)', className)} {...rest} />;
}
