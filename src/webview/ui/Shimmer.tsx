import type { ReactNode } from 'react';
import { cn } from './cn';

// Text shimmer for a running verb. The label is drawn twice: the muted copy stays put, a strong copy sits inside a
// masked window that slides across it while the copy slides back by the same amount. Both moves are transforms, so
// the sweep runs on the compositor and never repaints the transcript (animating background-position on a
// background-clip: text gradient re-painted the whole document layer every frame, ~3 ms on a 60k-node session).
// `active` false renders a plain span, so callers keep one element for both states (pass a real boolean: an explicit
// `undefined` takes the default and shimmers).
export function Shimmer({ active = true, className, children }: { active?: boolean; className?: string; children: ReactNode }) {
  if (!active) return <span className={className}>{children}</span>;
  return (
    <span className={cn('shimmer', className)}>
      {children}
      <span className="shimmer-sweep" aria-hidden="true"><span>{children}</span></span>
    </span>
  );
}
