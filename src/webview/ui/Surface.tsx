import type { HTMLAttributes, Ref } from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from './cn';

// Semantic surfaces own the frame; callers own layout and content density.
// Keeping this recipe separate from Card lets message and dock surfaces share
// the same border language without pretending they are the same component.
export const surfaceVariants = cva('min-w-0', {
  variants: {
    tone: {
      card: 'rounded-lg border border-card-line bg-card shadow-card',
      message: 'rounded-lg bg-bg-0 shadow-[inset_0_0_0_1px_var(--conversation-line)]',
      queue: 'rounded-md bg-(--cmp-bg) shadow-[inset_0_0_0_1px_var(--conversation-line)]',
      status: 'rounded-md bg-code',
    },
  },
});

export type SurfaceTone = NonNullable<VariantProps<typeof surfaceVariants>['tone']>;

export function Surface({ tone, className, ref, ...rest }: HTMLAttributes<HTMLDivElement> & {
  tone?: SurfaceTone;
  ref?: Ref<HTMLDivElement>;
}) {
  return <div ref={ref} className={cn(surfaceVariants({ tone }), className)} {...rest} />;
}
