import { useEffect, useRef, type ReactNode, type RefObject } from 'react';
import { Popover } from '../ui/Popover';
import { cn } from '../ui/cn';

interface CompletionListProps<T> {
  anchor: RefObject<HTMLElement | null>;
  items: T[];
  active: number;
  keyOf: (item: T) => string;
  // Shown instead of rows when there are none; without it an empty list renders nothing
  empty?: ReactNode;
  onHover: (index: number) => void;
  onPick: (item: T) => void;
  children: (item: T) => ReactNode;
}

// The inline completion shell shared by the @ file list and the / command list: as wide as the composer field, one --row per item,
// the active row highlighted. Keyboard handling stays in the textarea; the list only reflects the active row. Portals to the shell root
// like every overlay, since the composer's beam wrapper clips overflow
export function CompletionList<T>({ anchor, items, active, keyOf, empty, onHover, onPick, children }: CompletionListProps<T>) {
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const list = panel.current, item = list?.querySelector<HTMLElement>('[data-active]');
    if (!list || !item) return;
    const lr = list.getBoundingClientRect(), ir = item.getBoundingClientRect();
    if (ir.top < lr.top) list.scrollTop += ir.top - lr.top;
    else if (ir.bottom > lr.bottom) list.scrollTop += ir.bottom - lr.bottom;
  }, [active]);

  if (!items.length && !empty) return null;
  return <Popover.Root open>
    <Popover.Portal><Popover.Positioner anchor={anchor} width="anchor" side="top"
      // Base UI measures at temporary coordinates with opacity zero. Those rows
      // must not receive hover and change the keyboard selection before placement.
      render={attributes => <div {...attributes} style={{ ...attributes.style,
        pointerEvents: attributes.style?.opacity === 0 ? 'none' : attributes.style?.pointerEvents,
      }} />}>
      {/* The popup is the list itself: cap at min(8 rows, available height) so the two max-height rules don't collide. */}
      <Popover.Popup finalFocus={false} ref={panel} role="listbox" className="scroll-thin flex max-h-[min(var(--spacing-pop),var(--available-height))] flex-col overflow-y-auto">
      {!items.length && <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{empty}</div>}
      {items.map((item, i) => (
        <button
          key={keyOf(item)}
          type="button"
          role="option"
          aria-selected={i === active}
          data-active={i === active || undefined}
          // Layout can dispatch enter events under a stationary pointer. Only
          // deliberate mouse movement changes the keyboard's active result.
          onMouseMove={() => onHover(i)}
          // mousedown would blur the textarea before click fires; preventing it keeps the caret where the token is
          onMouseDown={e => e.preventDefault()}
          onClick={() => onPick(item)}
          className={cn('flex min-h-row w-full shrink-0 items-center gap-2 rounded-md px-2 text-left text-2 text-fg-1 outline-none transition-colors', i === active && 'bg-hover')}
        >
          {children(item)}
        </button>
      ))}
      </Popover.Popup>
    </Popover.Positioner></Popover.Portal>
  </Popover.Root>;
}
