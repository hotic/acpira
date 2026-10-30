import { useLayoutEffect, useRef, type CSSProperties, type ReactNode, type Ref, type TextareaHTMLAttributes } from 'react';
import { useMergedRefs } from '../ui/mergeRefs';
import { cn } from '../ui/cn';
import { spacedPinyinCommit } from '../ui/ime';
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
export function PromptInput({ ref, marks, className, value, onScroll, onCompositionUpdate, onCompositionEnd, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  ref?: Ref<HTMLTextAreaElement>; marks?: readonly CommandMark[]; value: string;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  const merged = useMergedRefs(input, ref);
  // The input method's last shown text, to tell a raw commit of it (spaces and all) from a chosen candidate
  const shown = useRef<string | undefined>(undefined);
  const commitEnd = (data: string) => {
    const committed = shown.current;
    shown.current = undefined;
    const fixed = spacedPinyinCommit(data, committed);
    if (fixed === undefined) return;
    // After the commit settles: WebKit fires compositionend before the input that inserts the text
    setTimeout(() => {
      const el = input.current;
      if (!el || el.selectionStart !== el.selectionEnd) return;
      const end = el.selectionStart;
      const start = end - data.length;
      if (start < 0 || el.value.slice(start, end) !== data) return;
      el.setSelectionRange(start, end);
      // insertText keeps the native undo stack and fires input, so the controlled value follows through onChange
      if (!document.execCommand('insertText', false, fixed)) el.setSelectionRange(end, end);
    }, 0);
  };
  const sync = () => {
    if (!input.current || !mirror.current) return;
    // clientWidth excludes the native scrollbar; both layers must wrap there.
    mirror.current.style.width = `${input.current.clientWidth}px`;
    mirror.current.scrollTop = input.current.scrollTop;
    mirror.current.scrollLeft = input.current.scrollLeft;
  };
  const hasMarks = !!marks?.length;
  const markKey = marks?.map(m => `${m.start}:${m.sigil ?? '/'}${m.name}`).join();
  useLayoutEffect(() => {
    if (!hasMarks) return;
    sync();
    const observer = new ResizeObserver(sync);
    if (input.current) observer.observe(input.current);
    return () => observer.disconnect();
  }, [hasMarks, value, markKey]);
  return <div className="relative min-w-0">
    {marks?.length ? <div ref={mirror} aria-hidden="true" className={cn(className,
      'pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap [overflow-wrap:break-word]',
    )}>
      {commandSegments(value, marks)}{'\u200b'}
    </div> : null}
    <textarea {...props} ref={merged} value={value} onScroll={e => { sync(); onScroll?.(e); }}
      onCompositionUpdate={e => { shown.current = e.data; onCompositionUpdate?.(e); }}
      onCompositionEnd={e => { commitEnd(e.data); onCompositionEnd?.(e); }}
      className={cn(className, 'relative block w-full', marks?.length && 'text-transparent caret-fg-strong selection:text-fg-strong')} />
  </div>;
}
