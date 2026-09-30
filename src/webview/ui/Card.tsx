import type { HTMLAttributes, Ref } from 'react';
import { cn } from './cn';
import { surfaceVariants } from './Surface';

// Container: the surface axis (hairline / tonal / stroke) decides border and background via the --card-* tokens
export function Card({ className, ...rest }: HTMLAttributes<HTMLDivElement> & { ref?: Ref<HTMLDivElement> }) {
  return <div className={cn(surfaceVariants({ tone: 'card' }), className)} {...rest} />;
}
