import { useLayoutEffect, useRef, type Ref, type TextareaHTMLAttributes } from 'react';
import { useMergedRefs } from '../ui/mergeRefs';
import { cn } from '../ui/cn';

export const COMMAND_MARK = 'rounded-sm -mx-1 px-1 py-0.5 bg-accent/15 text-accent [box-decoration-break:clone]';

// Keep the native textarea for selection, IME, undo, paste, and accessibility.
// Its mirror paints a command token without changing any character's geometry.
export function PromptInput({ ref, command, className, value, onScroll, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  ref?: Ref<HTMLTextAreaElement>; command?: string; value: string;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  const merged = useMergedRefs(input, ref);
  const sync = () => {
    if (!input.current || !mirror.current) return;
    // clientWidth excludes the native scrollbar; both layers must wrap there.
    mirror.current.style.width = `${input.current.clientWidth}px`;
    mirror.current.scrollTop = input.current.scrollTop;
    mirror.current.scrollLeft = input.current.scrollLeft;
  };
  useLayoutEffect(() => {
    sync();
    const observer = new ResizeObserver(sync);
    if (input.current) observer.observe(input.current);
    return () => observer.disconnect();
  }, [value, command]);
  return <div className="relative min-w-0">
    {command && <div ref={mirror} aria-hidden="true" className={cn(className,
      'pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap [overflow-wrap:break-word]',
    )}>
      <mark className={COMMAND_MARK}>/{command}</mark>{value.slice(command.length + 1)}{'\u200b'}
    </div>}
    <textarea {...props} ref={merged} value={value} onScroll={e => { sync(); onScroll?.(e); }}
      className={cn(className, 'relative block w-full', command && 'text-transparent caret-fg-strong selection:text-fg-strong')} />
  </div>;
}
