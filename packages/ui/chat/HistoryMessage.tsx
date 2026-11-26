import { createContext, memo, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { EditTurnRequest } from '@acpira/shared/protocol';
import type { Draft, SlashCommand, UserTurn } from '@acpira/shared/transcript';
import { captureTurnSettings, editTurnConfig, openTurnControls, refreshTurnControls } from '@acpira/shared/turnSettings';
import type { ModelShapes } from '@acpira/shared/modelShapes';
import { useAppearance } from '../appearance';
import { Composer, type ComposerProps } from './Composer';
import { EditAttachments } from './Attachments';
import { dropDraft, parkedDraft } from './useComposerDraft';
import { UserMessage } from './Turns';
import { cssTimeVar } from '../ui/cssTime';
import { Row } from '../ui/Row';
import { AgentMark } from './AgentMark';
import { firstPlainLine, SIDEKICK_NAME } from '@acpira/shared/subagents';
import { t, useLocale } from '../i18n';

// Every prompt card reads this, so it must stay stable across stream pushes: editability is a flag,
// and the composer props (which change with each chunk) travel in HistoryComposerContext for the mounted editor alone
interface HistoryContextValue {
  sessionId: string;
  edit: (request: EditTurnRequest) => Promise<void>;
  editing?: number;
  select: (index?: number, turnId?: string) => void;
  editable: boolean;
  // Remembered per-model parameters (shared/modelShapes.ts); changes only when the agent reveals a new model's shape
  shapes?: ModelShapes;
}

export const HistoryContext = createContext<HistoryContextValue | undefined>(undefined);
export const HistoryComposerContext = createContext<ComposerProps | undefined>(undefined);

// One frame per prompt, kept mounted while the card and its inline editor swap inside it: it carries the sticky positioning
// (so a card stuck at the top opens its editor right there instead of jumping back to its natural place). Opening and
// closing the editor never fades the card; only the opaque base's height glides back on close so the swap stays crisp.
// Sticky positioning never changes the card height; long prompts keep the same bounded, scrollable body.
export const HistoryMessage = memo(function HistoryMessage(p: {
  turn: UserTurn; index: number; turnIndex: number; blobUrl?: (blob: string) => string; commands?: readonly SlashCommand[]; summons?: readonly string[];
}) {
  // memo keeps a row from rendering again; the locale context still reaches it after a language change
  useLocale();
  const context = useContext(HistoryContext);
  const { motion } = useAppearance();
  const base = useRef<HTMLDivElement>(null);
  // Height measured right before swapping between card and inline editor; `editorHeight` keeps the editor's height so a
  // session patch arriving before editTurnResult (which auto-closes the editor on turn id change) still glides shut smoothly
  const from = useRef<number>(undefined);
  const editorHeight = useRef<number>(undefined);
  // Optimistic submitted view held only until the engine's replacement turn lands, preventing a brief flash of the old text
  const [submitted, setSubmitted] = useState<{ prev: UserTurn; turn: UserTurn }>();
  useEffect(() => {
    if (submitted && submitted.prev !== p.turn) setSubmitted(undefined);
  }, [submitted, p.turn]);
  const editor = context && context.editing === p.turnIndex ? context : undefined;
  const editing = !!editor;
  const select = (index?: number) => {
    from.current = base.current?.offsetHeight;
    context!.select(index, index !== undefined ? p.turn.id : undefined);
  };
  useLayoutEffect(() => {
    const el = base.current;
    if (editing && el) editorHeight.current = el.offsetHeight;
    const start = from.current ?? (!editing ? editorHeight.current : undefined);
    from.current = undefined;
    if (!editing) editorHeight.current = undefined;
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
  // An unsent edit closed by a click elsewhere keeps showing as edited (its text and the attachments it kept), so the
  // card never reads as the original until reopened. Read at render: closing the editor re-renders this card
  const pendingTurn = submitted?.prev === p.turn ? submitted.turn : undefined;
  const shown = editor || !context ? p.turn : pendingTurn ?? parkedTurn(p.turn, editKey(context.sessionId, p.turn, p.turnIndex));
  return (
    // The wrapper reaches one gap above the card (pt-gap, given back by -mt-gap) and sticks flush at the top: the card stops
    // below the thread's top fade (thread-fade), and the band, painted in the page's colour while stuck, hides the rows
    // that scrolled under the card. A card pushed out by the next exchange dissolves through the fade instead of a flat cut
    <div data-sticky-prompt className="peer/prompt pointer-events-none sticky top-0 z-10 -mt-gap flex min-w-0 shrink-0 flex-col pt-gap">
      <div ref={base} data-part="user-message" data-turn={p.turnIndex} className="pointer-events-auto flex min-w-0 flex-col rounded-lg bg-bg-0">
        <div className="flex min-w-0 flex-col">
          {editor
            ? <HistoryEditor
                key={editKey(editor.sessionId, p.turn, p.turnIndex)}
                {...p}
                context={editor}
                onSubmitting={() => { editorHeight.current = base.current?.offsetHeight; }}
                onSubmitted={turn => setSubmitted({ prev: p.turn, turn })}
                onClose={() => select(undefined)}
              />
            : <UserMessage {...p} turn={shown} onEdit={editable ? () => select(p.turnIndex) : undefined} />}
        </div>
      </div>
    </div>
  );
});

// An unsent history edit closed by a click elsewhere keeps its picks here (its text and new attachments park in the composer's
// draft store under the same key) and reopens with them; only Cancel / Escape or a send drops them. Webview memory only
const EDIT_PICKS = new Map<string, { controls: ComposerProps['controls']; retained: number[] }>();

function editKey(sessionId: string, turn: UserTurn, turnIndex: number) {
  return `edit:${sessionId}:${turn.id ?? `${turnIndex}:${turn.text}`}`;
}

// The closed card's view of a parked edit: its text, and the original attachments it still keeps. Attachments added in
// the editor are not shown, as the card renders only stored blobs
function parkedTurn(turn: UserTurn, key: string): UserTurn {
  const text = parkedDraft(key)?.text ?? turn.text;
  const retained = EDIT_PICKS.get(key)?.retained;
  const dropped = !!retained && retained.length < (turn.attachments?.length ?? 0);
  // Opening and closing without a change parks the original: keep the turn object itself
  if (text === turn.text && !dropped) return turn;
  return { ...turn, text, ...(dropped ? { attachments: turn.attachments!.filter((_, i) => retained!.includes(i)) } : {}) };
}

function HistoryEditor({ turn, turnIndex, blobUrl, context: c, onSubmitting, onSubmitted, onClose }: {
  turn: UserTurn; turnIndex: number; blobUrl?: (blob: string) => string; context: HistoryContextValue;
  onSubmitting: () => void; onSubmitted: (turn: UserTurn) => void; onClose: () => void;
}) {
  // Provided together with HistoryContext by the shell; the editor is the only reader
  const composer = useContext(HistoryComposerContext)!;
  const [key] = useState(() => editKey(c.sessionId, turn, turnIndex));
  const [picks, setControls] = useState(() => EDIT_PICKS.get(key)?.controls ?? openTurnControls(composer.controls, turn.settings, c.shapes));
  const controls = useMemo(() => refreshTurnControls(picks, composer.controls, c.shapes), [picks, composer.controls, c.shapes]);
  const [retained, setRetained] = useState(() => EDIT_PICKS.get(key)?.retained ?? (turn.attachments ?? []).map((_, i) => i));
  useEffect(() => { EDIT_PICKS.set(key, { controls, retained }); }, [key, controls, retained]);
  const discard = () => {
    EDIT_PICKS.delete(key);
    dropDraft(key);
  };
  const [turnCount] = useState(composer.turnCount ?? composer.turns?.length ?? 0);
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
    onSubmitting();
    setPending(true);
    try {
      await c.edit({ sessionId: c.sessionId, turnIndex, turnCount, originalText: turn.text, turnId: turn.id,
        text, attachments, retainedAttachments: retained, settings: captureTurnSettings(controls) });
      const keptAttachments = turn.attachments ? turn.attachments.filter((_, i) => retained.includes(i)) : undefined;
      onSubmitted({ ...turn, text, ...(keptAttachments ? { attachments: keptAttachments } : {}) });
      discard();
      onClose();
    } finally { setPending(false); }
  };
  return <div className="flex min-w-0 flex-col gap-gap" onPointerDownCapture={() => { insidePointer.current = true; }}>
    {/* A running turn does not lock the editor: sending stops that turn first (engine `edit_turn`). A failed send keeps
        the text here and reports through the composer's own toast, like the main composer, never as a line under the card */}
    <Composer {...composer} running={false} disabled={pending || composer.disabled}
      controls={controls} usage={undefined} draftKey={key}
      onSetMode={modeId => setControls(cur => ({ ...refreshTurnControls(cur, composer.controls, c.shapes), modeId }))}
      onSetConfig={(id, value) => setControls(cur => editTurnConfig(refreshTurnControls(cur, composer.controls, c.shapes), composer.controls, id, value, c.shapes))}
      edit={{ text: turn.text, hasAttachments: retained.length > 0, onCancel: () => { discard(); onClose(); }, dismissOnOutside: true,
        attachments: <EditAttachments attachments={turn.attachments ?? []} retained={retained} blobUrl={blobUrl} disabled={pending} onRemove={i => setRetained(r => r.filter(n => n !== i))} />,
      }}
      onSend={send}
    />
  </div>;
}

// The automatic message that hands summoned agents' replies (or their questions) back to the agent (relay.rs
// `wake_text`, one `## Persona (cli)` section each, `## Persona (cli) asks` for a question). The message is for the
// agent; the reader gets each reporter's return in the sidekick relay's handback shape: its mark, its name, "handed
// back" and the reply's first line. The sidekick's own return is already its relay's handback row, so a wake with only
// the sidekick shows nothing and its reply joins the turn above. It is known by its name alone, since it runs in
// whichever CLI its pick names
export function shownReports(text: string): { name: string; agent: string; asks: boolean; line?: string }[] {
  return [...text.matchAll(/^## (.+) \(([^()]*)\)( asks)?$/gm)].map((m, i, all) => {
    const body = text.slice(m.index! + m[0].length, all[i + 1]?.index ?? text.length);
    const line = firstPlainLine(body);
    return { name: m[1]!, agent: m[2]!, asks: !!m[3], line };
  }).filter(r => r.name !== SIDEKICK_NAME);
}

function SubagentsReported({ text }: { text: string }) {
  const reports = shownReports(text);
  if (!reports.length) return null;
  // Same inset and row height as an agent turn's rows (AgentMessage), so the lines stay inside the transcript column.
  // A seam inside one exchange, not a message: one row gap after the reply above (`-mt-msg-join`), and the reply it
  // wakes joins it the same way (`joined` in Shell.tsx)
  return (
    <div className="-mt-msg-join flex min-w-0 flex-col px-pad [--row:var(--chat-row)]">
      {reports.map((r, i) => (
        <Row key={i} lead={<AgentMark id={r.agent} name={r.name} className="size-icon" />}>
          <span className="flex min-w-0 items-baseline gap-gap">
            <span className="shrink-0 text-fg-1">{r.name}</span>
            <span className="shrink-0 text-fg-2">{t(r.asks ? 'chat.subagentAsks' : 'sidekick.handback')}</span>
            {r.line && <span className="min-w-0 truncate text-fg-3">{r.line}</span>}
          </span>
        </Row>
      ))}
    </div>
  );
}
