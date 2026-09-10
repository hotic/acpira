import { createContext, memo, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { EditTurnRequest } from '@shared/protocol';
import type { UserTurn } from '@shared/transcript';
import { captureTurnSettings, controlsForTurn } from '@shared/turnSettings';
import { useAppearance } from '../appearance';
import { Composer, type ComposerProps } from './Composer';
import { EditAttachments } from './Attachments';
import { UserMessage } from './Turns';
import { promptIsStuck, scrollerUsable } from './promptStuck';
import { cn } from '../ui/cn';

// Every prompt card reads this, so it must stay stable across stream pushes: editability is a flag,
// and the composer props (which change with each chunk) travel in HistoryComposerContext for the mounted editor alone
interface HistoryContextValue {
  sessionId: string;
  edit: (request: EditTurnRequest) => Promise<void>;
  editing?: number;
  select: (index?: number) => void;
  editable: boolean;
}

export const HistoryContext = createContext<HistoryContextValue | undefined>(undefined);
export const HistoryComposerContext = createContext<ComposerProps | undefined>(undefined);

// One frame per prompt, kept mounted while the card and its inline editor swap inside it: it carries the sticky positioning
// (so a card stuck at the top opens its editor right there instead of jumping back to its natural place). Opening the editor is
// instant — a click should land in the text at once, like direct manipulation; only the way back (cancel / sent) animates: the
// opaque base animates its own height while the returning card fades in. Automatic prompts are plain rows. Keep the base outside
// the fade so replies cannot show through the editor or the swapping content.
// A stuck card folds to a few lines: the sentinel at the exchange's top leaving the scroller marks the stuck state, and the frame
// keeps its natural flow height meanwhile so the fold never moves the conversation under the reader; the freed area is transparent
// and lets pointer events through to the reply scrolling beneath it. A hidden sidebar webview collapses the thread to no box —
// those IntersectionObserver records are ignored, then re-checked when the thread is visible again.
export const HistoryMessage = memo(function HistoryMessage(p: { turn: UserTurn; index: number; turnIndex: number; blobUrl?: (blob: string) => string }) {
  const context = useContext(HistoryContext);
  const { motion } = useAppearance();
  const frame = useRef<HTMLDivElement>(null);
  const base = useRef<HTMLDivElement>(null);
  // Height measured right before an animated swap; the layout effect animates from it once the replacement has laid out
  const from = useRef<number>(undefined);
  // Swap counter remounts the content; `fade` is true only for the way back, so opening the editor never fades
  const [swap, setSwap] = useState({ n: 0, fade: false });
  const [stuck, setStuck] = useState(false);
  const sentinel = useCallback((el: HTMLDivElement | null) => {
    if (!el || typeof IntersectionObserver === 'undefined') return;
    const thread = el.closest('[data-thread]');
    const apply = (entry: IntersectionObserverEntry) => {
      const below = promptIsStuck(entry);
      if (below === undefined) return;
      const target = frame.current;
      // Measure before React applies the fold, while the frame still has its natural height.
      if (target) target.style.minHeight = below ? `${target.offsetHeight}px` : '';
      setStuck(below);
    };
    const observer = new IntersectionObserver(([entry]) => { if (entry) apply(entry); }, { root: thread, threshold: 0 });
    observer.observe(el);
    if (!(thread instanceof HTMLElement)) return () => observer.disconnect();
    // A collapsed webview never delivers a usable record; force one when the thread gets a box again.
    let usable = scrollerUsable(thread);
    const ro = new ResizeObserver(() => {
      const next = scrollerUsable(thread);
      if (next === usable) return;
      usable = next;
      if (!next) return;
      observer.unobserve(el);
      observer.observe(el);
    });
    ro.observe(thread);
    return () => { observer.disconnect(); ro.disconnect(); };
  }, []);
  const editor = context && context.editing === p.turnIndex ? context : undefined;
  const editing = !!editor;
  const select = (index?: number) => {
    const closing = index === undefined;
    from.current = closing ? base.current?.offsetHeight : undefined;
    setSwap(s => ({ n: s.n + 1, fade: closing }));
    context!.select(index);
  };
  useLayoutEffect(() => {
    const el = base.current;
    const start = from.current;
    from.current = undefined;
    if (!el || start === undefined) return;
    const end = el.offsetHeight;
    if (start === end || motion === 'none' || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const duration = parseFloat(getComputedStyle(el).getPropertyValue('--dur-open')) || 0;
    el.style.overflow = 'hidden';
    const animation = el.animate([{ height: `${start}px` }, { height: `${end}px` }], { duration, easing: 'cubic-bezier(0.2, 0.7, 0.2, 1)' });
    const done = () => { el.style.overflow = ''; };
    animation.onfinish = done;
    animation.oncancel = done;
    return () => animation.cancel();
  }, [editing, motion]);
  if (p.turn.auto) return <UserMessage {...p} />;
  const editable = !!context?.editable;
  return (
    <>
      <div ref={sentinel} aria-hidden="true" className="pointer-events-none absolute top-0 left-0 size-px" />
      <div ref={frame} className="pointer-events-none sticky top-0 z-10 flex min-w-0 shrink-0 flex-col">
        <div ref={base} className="pointer-events-auto flex min-w-0 flex-col rounded-lg bg-bg-0">
          <div key={swap.n} className={cn('flex min-w-0 flex-col', swap.fade && 'fade-in')}>
            {editor
              ? <HistoryEditor {...p} context={editor} onClose={() => select(undefined)} />
              : <UserMessage {...p} compact={stuck} onEdit={editable ? () => select(p.turnIndex) : undefined} />}
          </div>
        </div>
      </div>
    </>
  );
});

function HistoryEditor({ turn, turnIndex, blobUrl, context: c, onClose }: {
  turn: UserTurn; turnIndex: number; blobUrl?: (blob: string) => string; context: HistoryContextValue; onClose: () => void;
}) {
  // Provided together with HistoryContext by the shell; the editor is the only reader
  const composer = useContext(HistoryComposerContext)!;
  const [controls, setControls] = useState(() => controlsForTurn(composer.controls, turn.settings));
  const [retained, setRetained] = useState(() => (turn.attachments ?? []).map((_, i) => i));
  const [turnCount] = useState(composer.turns.length);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const insidePointer = useRef(false);
  useEffect(() => {
    const outside = () => {
      // React capture includes portaled model menus and image previews; DOM containment does not.
      const inside = insidePointer.current;
      insidePointer.current = false;
      if (!inside && !pending) onClose();
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [onClose, pending]);
  return <div className="flex min-w-0 flex-col gap-gap" onPointerDownCapture={() => { insidePointer.current = true; }}>
    <Composer {...composer} running={false} disabled={pending || composer.disabled || composer.running}
      controls={controls} usage={undefined}
      onNotice={setError}
      onSetMode={modeId => setControls(c => ({ ...c, modeId }))}
      onSetConfig={(id, value) => setControls(c => ({ ...c, options: c.options.map(o => o.id === id ? { ...o, value } : o) }))}
      edit={{ text: turn.text, hasAttachments: retained.length > 0, onCancel: onClose, dismissOnOutside: true,
        attachments: <EditAttachments attachments={turn.attachments ?? []} retained={retained} blobUrl={blobUrl} disabled={pending} onRemove={i => setRetained(r => r.filter(n => n !== i))} />,
      }}
      onSend={async (text, attachments) => {
        setError(undefined);
        setPending(true);
        try {
          await c.edit({ sessionId: c.sessionId, turnIndex, turnCount, originalText: turn.text, turnId: turn.id,
            text, attachments, retainedAttachments: retained, settings: captureTurnSettings(controls) });
          onClose();
        } finally { setPending(false); }
      }}
    />
    {error && <p role="alert" className="px-pad text-2 text-danger">{error}</p>}
  </div>;
}
