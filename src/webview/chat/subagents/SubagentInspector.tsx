import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowLeft, Network, Square, Unplug, X } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import type { QuestionBlock, TextBlock, Turn } from '@shared/transcript';
import { t } from '../../i18n';
import { cn } from '../../ui/cn';
import { IconButton } from '../../ui/Button';
import { Row } from '../../ui/Row';
import { Shimmer } from '../../ui/Shimmer';
import { AgentMessage, UserMessage } from '../Turns';
import { Questions, type OnAnswer } from '../Questions';
import { Prose } from '../Prose';
import { HistoryContext } from '../HistoryMessage';
import { TurnActionsContext } from '../TurnActions';
import { scrollerUsable } from '../promptStuck';
import { subagentTitle } from './subagentState';

interface InspectorProps {
  node: SubagentSummary;
  // The observed child's transcript; absent until the host answers observeSubagent
  transcript?: { turns: Turn[]; running: boolean; rev: number };
  onClose: () => void;
  // Opens the session's summon graph, the one place relations between children are drawn
  onGraph?: () => void;
  onCancel?: () => void;
  onPermission: (blockId: string, optionId: string) => void;
  // The child's open question card — the overlay makes the main thread inert, so the card renders here too
  question?: QuestionBlock;
  onAnswer?: OnAnswer;
  mode: 'docked' | 'overlay';
  blobUrl?: (blob: string) => string;
}

// Read-only drill-down for one delegated child: the parent's delegation row opens this, and nothing
// inside it controls the session except the declared cancel affordance.
export function SubagentInspector(p: InspectorProps) {
  const { node, mode } = p;
  const root = useRef<HTMLDivElement>(null);
  // Captured at render time: child effects (the question card) run before ours and steal focus,
  // so by mount time activeElement is already inside this pane.
  const opener = useRef<Element | null>(document.activeElement);
  useEffect(() => { root.current?.focus({ preventScroll: true }); }, [node.id]);
  useEffect(() => () => {
    const el = opener.current;
    // The inert ancestor clears in the same commit — the next frame is the first moment focus can land
    if (el instanceof HTMLElement && el.isConnected) requestAnimationFrame(() => el.focus({ preventScroll: true }));
  }, []);
  const title = subagentTitle(node, t);
  const cancelable = node.state === 'running' && node.controls.cancel && p.onCancel !== undefined;
  const cancelLabel = t(node.cancelRequested ? 'subagents.cancelling' : 'subagents.cancel');
  return (
    <div
      ref={root}
      tabIndex={-1}
      className="flex h-full min-h-0 flex-col outline-none"
      onKeyDown={e => {
        if (e.key === 'Escape' && !e.defaultPrevented) { e.stopPropagation(); p.onClose(); }
      }}
    >
      {/* Same shape as the main Header: plain title, actions on the right; ancestry lives in the summon graph */}
      <div className="flex h-hdr shrink-0 items-center gap-gap px-pad shadow-[inset_0_-1px_0_0_var(--line)]">
        <IconButton onClick={p.onClose} aria-label={t('subagents.back')} title={t('subagents.back')}>
          {mode === 'overlay' ? <ArrowLeft strokeWidth={1.5} /> : <X strokeWidth={1.5} />}
        </IconButton>
        <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-strong" title={title}>{title}</span>
        {cancelable && (
          <IconButton onClick={p.onCancel} disabled={node.cancelRequested} aria-label={cancelLabel} title={cancelLabel}
            className="disabled:cursor-not-allowed disabled:opacity-50">
            <Square strokeWidth={1.5} />
          </IconButton>
        )}
        {p.onGraph && (
          <IconButton onClick={p.onGraph} aria-haspopup="dialog" aria-label={t('subagents.graph')} title={t('subagents.graph')}>
            <Network strokeWidth={1.5} />
          </IconButton>
        )}
      </div>
      <SessionTab {...p} />
    </div>
  );
}

function SessionTab({ node, transcript, onPermission, question, onAnswer, blobUrl }: InspectorProps) {
  const scroll = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  // Stick to the bottom while the child streams, exactly like the main thread; scrolling up releases the follow
  useEffect(() => {
    const el = scroll.current;
    if (!el) return;
    const onScroll = () => { if (scrollerUsable(el)) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48; };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, []);
  useLayoutEffect(() => {
    const el = scroll.current;
    if (el && scrollerUsable(el) && pinned.current) el.scrollTop = el.scrollHeight;
  }, [transcript?.turns, transcript?.running]);
  const lastAgent = transcript ? transcript.turns.reduce((at, turn, i) => (turn.role === 'agent' ? i : at), -1) : -1;
  return (
    <>
      <div ref={scroll} className="scroll-thin min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="flex min-w-0 flex-col gap-msg px-pad py-pad">
          {node.task !== undefined && <TaskCard task={node.task} />}
          {node.visibility === 'receipt' ? (
            <>
              <p className="m-0 text-2 text-fg-3">{t('subagents.receiptOnly')}</p>
              {node.result !== undefined && (
                <section className="flex min-w-0 flex-col gap-1">
                  <div className="text-3 text-fg-3">{t('subagents.result')}</div>
                  <Prose block={textOf(node.result)} />
                </section>
              )}
            </>
          ) : transcript === undefined ? (
            <Row className="text-fg-3"><Shimmer active>{t('subagents.loading')}</Shimmer></Row>
          ) : (
            <TurnActionsContext.Provider value={undefined}>
              <HistoryContext.Provider value={undefined}>
                {/* Turns carry their own column padding, like in the main thread */}
                <div className="-mx-pad flex min-w-0 flex-col gap-msg">
                  {transcript.turns.map((turn, ti) => turn.role === 'agent'
                    ? (
                      <AgentMessage
                        key={ti}
                        turn={turn}
                        index={ti}
                        running={!!transcript.running && ti === lastAgent}
                        onPermission={onPermission}
                        memoryKey={`sub:${node.id}:${ti}`}
                        turnIndex={ti}
                        last={ti === transcript.turns.length - 1}
                        actions={false}
                        lead="static"
                      />
                    )
                    : <UserMessage key={ti} turn={turn} index={ti} blobUrl={blobUrl} />)}
                  {transcript.turns.length === 0 && <Row className="text-fg-3"><Shimmer active>{t('subagents.loading')}</Shimmer></Row>}
                </div>
              </HistoryContext.Provider>
            </TurnActionsContext.Provider>
          )}
          {node.visibility === 'nested' && node.result !== undefined && (
            <section className="flex min-w-0 flex-col gap-1">
              <div className="text-3 text-fg-3">{t('subagents.result')}</div>
              <Prose block={textOf(node.result)} />
            </section>
          )}
          {node.state === 'disconnected' && (
            <Row lead={<Unplug className="size-icon" strokeWidth={1.5} />} className="text-fg-3">
              <span className="min-w-0">{t('subagents.disconnectedNote')}</span>
            </Row>
          )}
        </div>
      </div>
      {question !== undefined && onAnswer !== undefined && <Questions block={question} onAnswer={onAnswer} />}
    </>
  );
}

function textOf(markdown: string): TextBlock {
  return { type: 'text', markdown };
}

// The delegated task in a quiet card; long briefs clamp to six lines with a text toggle
function TaskCard({ task }: { task: string }) {
  const body = useRef<HTMLDivElement>(null);
  const [clamped, setClamped] = useState(false);
  const [open, setOpen] = useState(false);
  useLayoutEffect(() => {
    const el = body.current;
    if (el) setClamped(el.scrollHeight > el.clientHeight + 1);
  }, [task]);
  return (
    <section className="flex min-w-0 flex-col gap-1">
      <div className="text-3 text-fg-3">{t('subagents.task')}</div>
      <div className="rounded-md border border-line bg-bg-1 p-pad">
        <div ref={body} className={cn('text-2 text-fg-2 whitespace-pre-wrap [overflow-wrap:anywhere]', !open && 'line-clamp-6')}>{task}</div>
        {(clamped || open) && (
          <button type="button" onClick={() => setOpen(o => !o)} className="mt-1 cursor-pointer text-3 text-fg-3 transition-colors hover:text-fg-1">
            {t(open ? 'subagents.taskCollapse' : 'subagents.taskExpand')}
          </button>
        )}
      </div>
    </section>
  );
}
