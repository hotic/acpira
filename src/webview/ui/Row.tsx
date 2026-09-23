import { createContext, useContext, useEffect, useRef, useState, type HTMLAttributes, type ReactNode, type Ref } from 'react';
import { cn } from './cn';
import { useMergedRefs } from './mergeRefs';
import { cva } from 'class-variance-authority';
import { Shimmer } from './Shimmer';

// Scope entrance effects to live transcript rows; menus and restored history stay still.
export const RowEntranceContext = createContext(false);

// Identities whose rows already entered. A tool changes component shape as it progresses (a running read is a
// ToolCall, a completed one joins a ReadGroup), and the remounted rows replayed their entrance mid-turn
const entered = new Set<string>();
const ENTERED_CAP = 4000;
// The live turn and its name, set once per message: `RowEntranceContext` gets narrowed below by EntranceOnce,
// and agents may number tool calls per session, so ids alone repeat across turns
export const EntranceScopeContext = createContext<{ live: boolean; scope: string }>({ live: false, scope: '' });

// Rows under `id` enter only with the first mount of that identity in a live turn. Rows that mount later under
// the same instance (a branch that changes shape as the tool progresses) stay still; content that genuinely
// appears later gets its own EntranceOnce
export function EntranceOnce({ id, children }: { id: string; children: ReactNode }) {
  const { live, scope } = useContext(EntranceScopeContext);
  const [enter, setEnter] = useState(() => {
    const key = `${scope}\u0000${id}`;
    const fresh = !entered.has(key);
    if (entered.size >= ENTERED_CAP) entered.clear();
    entered.add(key);
    return live && fresh;
  });
  // Rows read the context once, at their own mount: the ones mounted with this commit keep their entrance
  useEffect(() => { if (enter) setEnter(false); }, [enter]);
  return <RowEntranceContext.Provider value={enter}>{children}</RowEntranceContext.Provider>;
}

// The one shared "row": thought / plan / tool / status / session items all grow on this row.
// Row height --row; lead slot --lead (icon 14 or Orb 20 centered); label area gap --gap; trailing meta right-aligned.
export interface RowProps extends Omit<HTMLAttributes<HTMLElement>, 'children'> {
  ref?: Ref<HTMLElement>;
  lead?: ReactNode;
  trailing?: ReactNode;
  children?: ReactNode;
  interactive?: boolean;
  as?: 'div' | 'button';
  className?: string;
  dense?: boolean;
  tone?: 'action';
}

const rowVariants = cva('flex items-center gap-gap text-2 text-fg-2 select-none list-none text-left', {
  variants: {
    tone: { action: 'action-row' },
    dense: { true: 'min-h-row-dense', false: 'min-h-row' },
    interactive: { true: '-mx-hit px-hit cursor-pointer rounded-md hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 transition-colors' },
    enter: { true: 'process-row-enter' },
  },
});

export function Row({ lead, trailing, children, interactive, as = 'div', className, dense, tone, ref, ...rest }: RowProps) {
  const Tag = as;
  const live = useContext(RowEntranceContext);
  const [enter, setEnter] = useState(live);
  const self = useRef<HTMLElement>(null);
  const merged = useMergedRefs(self, ref);
  // Drop the entrance class once it has played: a hidden webview (display: none) restarts every CSS animation when
  // it is shown again, so a class left behind replays the fade on each return to the window. The rail segment
  // beside the row shares the keyframes, so the sibling animations are awaited as well
  useEffect(() => {
    if (!enter) return;
    const scope = self.current?.parentElement ?? self.current;
    const pending = scope?.getAnimations?.({ subtree: true })
      .filter(a => a instanceof CSSAnimation && a.animationName === 'acp-row-part-in' && a.playState !== 'finished') ?? [];
    if (!pending.length) { setEnter(false); return; }
    let disposed = false;
    void Promise.allSettled(pending.map(a => a.finished)).then(() => { if (!disposed) setEnter(false); });
    return () => { disposed = true; };
  }, [enter]);
  return (
    <Tag
      ref={merged as Ref<HTMLButtonElement & HTMLDivElement>}
      {...(as === 'button' ? { type: 'button' } : {})}
      className={cn(
        rowVariants({ dense: !!dense, interactive, enter, tone }),
        className,
      )}
      {...rest}
    >
      <span className={cn('row-lead size-lead shrink-0 items-center justify-center text-fg-3', lead === undefined ? 'row-lead-empty hidden' : 'flex')}>{lead}</span>
      <span className="row-content flex min-w-0 flex-1 items-baseline gap-2">{children}</span>
      {trailing !== undefined && <span className="row-trailing ml-auto flex shrink-0 items-center gap-2 text-3 text-fg-3 tabular-nums">{trailing}</span>}
    </Tag>
  );
}

// Keep short labels intact; the adjacent target gives up space and truncates first. `shimmer` marks the running verb.
export function RowLabel({ children, className, shimmer }: { children: ReactNode; className?: string; shimmer?: boolean }) {
  return <Shimmer active={!!shimmer} className={cn('shrink-0 whitespace-nowrap', className)}>{children}</Shimmer>;
}

// Target text within a row (file name / command), one step brighter than the verb
export function RowTarget({ children, mono, className }: { children: ReactNode; mono?: boolean; className?: string }) {
  return <span className={cn('row-target truncate text-fg-1/85', mono && 'font-mono text-mono', className)}>{children}</span>;
}
