import { createContext, memo, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { EditTurnRequest } from '@acpira/shared/protocol';
import type { Draft, SlashCommand, UserTurn } from '@acpira/shared/transcript';
import { captureTurnSettings, editTurnConfig, openTurnControls } from '@acpira/shared/turnSettings';
import type { ModelShapes } from '@acpira/shared/modelShapes';
import { useAppearance } from '../appearance';
import { Composer, type ComposerProps } from './Composer';
import { EditAttachments } from './Attachments';
import { dropDraft } from './useComposerDraft';
import { UserMessage } from './Turns';
import { cn } from '../ui/cn';
import { cssTimeVar } from '../ui/cssTime';
import { Row, RowLabel, RowTarget } from '../ui/Row';
import { AtSign } from 'lucide-react';
import { t, useLocale } from '../i18n';

// Every prompt card reads this, so it must stay stable across stream pushes: editability is a flag,
// and the composer props (which change with each chunk) travel in HistoryComposerContext for the mounted editor alone
interface HistoryContextValue {
  sessionId: string;
  edit: (request: EditTurnRequest) => Promise<void>;
  editing?: number;
  select: (index?: number) => void;
  editable: boolean;
  // Remembered per-model parameters (shared/modelShapes.ts); changes only when the agent reveals a new model's shape
  shapes?: ModelShapes;
}

export const HistoryContext = createContext<HistoryContextValue | undefined>(undefined);
export const HistoryComposerContext = createContext<ComposerProps | undefined>(undefined);

// One frame per prompt, kept mounted while the card and its inline editor swap inside it: it carries the sticky positioning
// (so a card stuck at the top opens its editor right there instead of jumping back to its natural place). Opening the editor is
// instant — a click should land in the text at once, like direct manipulation; only the way back (cancel / sent) animates: the
// opaque base animates its own height while the returning card fades in. Automatic prompts render nothing. Keep the base outside
// the fade so replies cannot show through the editor or the swapping content.
// Sticky positioning never changes the card height; long prompts keep the same bounded, scrollable body.
export const HistoryMessage = memo(function HistoryMessage(p: {
  turn: UserTurn; index: number; turnIndex: number; blobUrl?: (blob: string) => string; commands?: readonly SlashCommand[]; summons?: readonly string[];
}) {
  // memo keeps a row from rendering again; the locale context still reaches it after a language change
  useLocale();
  const context = useContext(HistoryContext);
  const { motion } = useAppearance();
  const base = useRef<HTMLDivElement>(null);
  // Height measured right before an animated swap; the layout effect animates from it once the replacement has laid out
  const from = useRef<number>(undefined);
  // Swap counter remounts the content; `fade` is true only for the way back, so opening the editor never fades
  const [swap, setSwap] = useState({ n: 0, fade: false });
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
    const duration = cssTimeVar(el, '--dur-open', 0);
    el.style.overflow = 'hidden';
    const animation = el.animate([{ height: `${start}px` }, { height: `${end}px` }], { duration, easing: 'cubic-bezier(0.2, 0.7, 0.2, 1)' });
    const done = () => { el.style.overflow = ''; };
    animation.onfinish = done;
    animation.oncancel = done;
    return () => animation.cancel();
  }, [editing, motion]);
  if (p.turn.auto) return p.turn.autoReason === 'subagents' ? <SubagentsReported text={p.turn.text} /> : null;
  const editable = !!context?.editable;
  return (
    <div data-sticky-prompt className="peer/prompt pointer-events-none sticky top-0 z-10 flex min-w-0 shrink-0 flex-col">
      <div ref={base} data-part="user-message" className="pointer-events-auto flex min-w-0 flex-col rounded-lg bg-bg-0">
        <div key={swap.n} className={cn('flex min-w-0 flex-col', swap.fade && 'fade-in')}>
          {editor
            ? <HistoryEditor {...p} context={editor} onClose={() => select(undefined)} />
            : <UserMessage {...p} onEdit={editable ? () => select(p.turnIndex) : undefined} />}
        </div>
      </div>
    </div>
  );
});

// An unsent history edit closed by a click elsewhere keeps its picks here (its text and new attachments park in the composer's
// draft store under the same key) and reopens with them; only Cancel / Escape or a send drops them. Webview memory only
const EDIT_PICKS = new Map<string, { controls: ComposerProps['controls']; retained: number[] }>();

function HistoryEditor({ turn, turnIndex, blobUrl, context: c, onClose }: {
  turn: UserTurn; turnIndex: number; blobUrl?: (blob: string) => string; context: HistoryContextValue; onClose: () => void;
}) {
  // Provided together with HistoryContext by the shell; the editor is the only reader
  const composer = useContext(HistoryComposerContext)!;
  const [key] = useState(() => `edit:${c.sessionId}:${turn.id ?? `${turnIndex}:${turn.text}`}`);
  const [controls, setControls] = useState(() => EDIT_PICKS.get(key)?.controls ?? openTurnControls(composer.controls, turn.settings, c.shapes));
  const [retained, setRetained] = useState(() => EDIT_PICKS.get(key)?.retained ?? (turn.attachments ?? []).map((_, i) => i));
  useEffect(() => { EDIT_PICKS.set(key, { controls, retained }); }, [key, controls, retained]);
  const discard = () => {
    EDIT_PICKS.delete(key);
    dropDraft(key);
  };
  const [turnCount] = useState(composer.turnCount ?? composer.turns?.length ?? 0);
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
  const send = async (text: string, attachments: Draft[]) => {
    setError(undefined);
    setPending(true);
    try {
      await c.edit({ sessionId: c.sessionId, turnIndex, turnCount, originalText: turn.text, turnId: turn.id,
        text, attachments, retainedAttachments: retained, settings: captureTurnSettings(controls) });
      discard();
      onClose();
    } finally { setPending(false); }
  };
  return <div className="flex min-w-0 flex-col gap-gap" onPointerDownCapture={() => { insidePointer.current = true; }}>
    {/* A running turn does not lock the editor: sending stops that turn first (engine `edit_turn`) */}
    <Composer {...composer} running={false} disabled={pending || composer.disabled}
      controls={controls} usage={undefined} draftKey={key}
      onNotice={setError}
      onSetMode={modeId => setControls(c => ({ ...c, modeId }))}
      onSetConfig={(id, value) => setControls(cur => editTurnConfig(cur, composer.controls, id, value, c.shapes))}
      edit={{ text: turn.text, hasAttachments: retained.length > 0, onCancel: () => { discard(); onClose(); }, dismissOnOutside: true,
        attachments: <EditAttachments attachments={turn.attachments ?? []} retained={retained} blobUrl={blobUrl} disabled={pending} onRemove={i => setRetained(r => r.filter(n => n !== i))} />,
      }}
      onSend={send}
    />
    {error && <p role="alert" className="px-pad text-2 text-danger">{error}</p>}
  </div>;
}

// The automatic message that hands summoned agents' replies back to the agent (relay.rs `wake_text`): the message itself
// is for the agent, the user reads one line naming who reported, from its `## Persona (cli)` headings
function SubagentsReported({ text }: { text: string }) {
  const names = [...text.matchAll(/^## (.+?)(?: \([^()]*\))?$/gm)].map(m => m[1]!);
  // Same inset and row height as an agent turn's rows (AgentMessage), so the line stays inside the transcript column.
  // It is a seam inside one exchange, not a message: one row gap after the reply above (`-mt-msg-join`), and the reply
  // it wakes joins it the same way (`joined` in Shell.tsx)
  return (
    <div className="-mt-msg-join flex min-w-0 flex-col px-pad [--row:var(--chat-row)]">
      <Row lead={<AtSign className="size-icon" strokeWidth={1.5} />}>
        <RowLabel>{t('chat.subagentsReported')}</RowLabel>
        {names.length > 0 && <RowTarget>{names.join(', ')}</RowTarget>}
      </Row>
    </div>
  );
}
