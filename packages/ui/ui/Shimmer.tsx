import { useLayoutEffect, useRef, type ReactNode } from 'react';
import { cn } from './cn';

// Text shimmer for a running verb: both copies stay put while a masked band sweeps across the strong copy (motion.css).
// `active` false renders a plain span, so callers keep one element for both states (pass a real boolean: an explicit
// `undefined` takes the default and shimmers).
export function Shimmer({ active = true, sync = false, className, children }: { active?: boolean; sync?: boolean; className?: string; children: ReactNode }) {
  const clip = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    if (!active || !sync) return;
    // Tool headings can remount when a call gains details or joins a group. Anchor their sweep to the document's
    // timeline so the new shape continues the same phase instead of restarting from the left edge.
    for (const animation of clip.current?.getAnimations({ subtree: true }) ?? []) {
      if (animation instanceof CSSAnimation && animation.animationName.startsWith('acp-shimmer-')) animation.startTime = 0;
    }
  }, [active, sync]);
  if (!active) return <span className={className}>{children}</span>;
  return (
    <span className={cn('shimmer', className)}>
      {children}
      <span ref={clip} className="shimmer-clip" aria-hidden="true"><span className="shimmer-band"><span className="shimmer-copy"><span>{children}</span></span></span></span>
    </span>
  );
}
