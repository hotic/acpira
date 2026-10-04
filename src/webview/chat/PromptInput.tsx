import { useLayoutEffect, useRef, type CSSProperties, type ReactNode, type Ref, type TextareaHTMLAttributes } from 'react';
import { useMergedRefs } from '../ui/mergeRefs';
import { cn } from '../ui/cn';
import { markRoom, type CommandMark } from './slashCommands';

// The pill's horizontal overhang comes from `.prompt-mark` (--mark-room, see markRoom)
export const COMMAND_MARK = 'prompt-mark rounded-sm py-0.5 bg-command/15 text-command [box-decoration-break:clone]';
// A summoned subagent persona (`@name`): the same pill in its own colour, so a summon never reads as a command
export const SUMMON_MARK = 'prompt-mark rounded-sm py-0.5 bg-summon/15 text-summon [box-decoration-break:clone]';

// The text broken around the marks into plain runs and <mark> pills, for the composer mirror and the sent user message alike
export function commandSegments(value: string, marks: readonly CommandMark[]): ReactNode[] {
  const segments: ReactNode[] = [];
  let at = 0;
  marks.forEach((m, i) => {
    const sigil = m.sigil ?? '/';
    const room = markRoom(value, marks, i);
    segments.push(value.slice(at, m.start), <mark key={m.start} className={sigil === '@' ? SUMMON_MARK : COMMAND_MARK}
      style={room ? { '--mark-room': room } as CSSProperties : undefined}>{sigil}{m.name}</mark>);
    at = m.start + m.name.length + 1;
  });
  segments.push(value.slice(at));
  return segments;
}

// Keep the native textarea for selection, IME, undo, paste, and accessibility.
// Its mirror paints command and summon tokens without changing any character's geometry.
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
  const markKey = marks?.map(m => `${m.start}:${m.sigil ?? '/'}${m.name}`).join();
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
