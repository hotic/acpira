import { useLayoutEffect, useRef, type ReactNode, type Ref, type TextareaHTMLAttributes } from 'react';
import { useMergedRefs } from '../ui/mergeRefs';
import { cn } from '../ui/cn';
import type { CommandMark } from './slashCommands';

export const COMMAND_MARK = 'rounded-sm -mx-1 px-1 py-0.5 bg-command/15 text-command [box-decoration-break:clone]';

// The text broken around the marks into plain runs and <mark> pills, for the composer mirror and the sent user message alike
export function commandSegments(value: string, marks: readonly CommandMark[]): ReactNode[] {
  const segments: ReactNode[] = [];
  let at = 0;
  for (const m of marks) {
    segments.push(value.slice(at, m.start), <mark key={m.start} className={COMMAND_MARK}>/{m.name}</mark>);
    at = m.start + m.name.length + 1;
  }
  segments.push(value.slice(at));
  return segments;
}

// Keep the native textarea for selection, IME, undo, paste, and accessibility.
// Its mirror paints command tokens without changing any character's geometry.
export function PromptInput({ ref, marks, className, value, onScroll, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  ref?: Ref<HTMLTextAreaElement>; marks?: readonly CommandMark[]; value: string;
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
  const markKey = marks?.map(m => `${m.start}:${m.name}`).join();
  useLayoutEffect(() => {
    sync();
    const observer = new ResizeObserver(sync);
    if (input.current) observer.observe(input.current);
    return () => observer.disconnect();
  }, [value, markKey]);
  return <div className="relative min-w-0">
    {marks?.length ? <div ref={mirror} aria-hidden="true" className={cn(className,
      'pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap [overflow-wrap:break-word]',
    )}>
      {commandSegments(value, marks)}{'\u200b'}
    </div> : null}
    <textarea {...props} ref={merged} value={value} onScroll={e => { sync(); onScroll?.(e); }}
      className={cn(className, 'relative block w-full', marks?.length && 'text-transparent caret-fg-strong selection:text-fg-strong')} />
  </div>;
}
