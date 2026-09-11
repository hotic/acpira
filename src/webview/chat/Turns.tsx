import { Fragment, memo, useCallback, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { Check, ChevronRight, Compass, Hand, MessageCircleQuestion, TriangleAlert, X } from 'lucide-react';
import type { AgentBlock, AgentTurn, CompactionBlock, PermissionBlock, ToolCallBlock, ToolKind, UserTurn } from '@shared/transcript';
import { useAppearance, type Appearance } from '../appearance';
import { getLocale, t } from '../i18n';
import { COMMAND_MARK } from './PromptInput';
import { turnOutcome } from './turnOutcome';
import { Row, RowLabel, RowTarget, RowEntranceContext } from '../ui/Row';
import { Shimmer } from '../ui/Shimmer';
import { Disclosure, DisclosureObserverContext } from '../ui/Disclosure';
import { Collapsible } from '../ui/Collapsible';
import { Orb } from '../effects/Orb';
import { cn } from '../ui/cn';
import { useMergedRefs } from '../ui/mergeRefs';
import { useScrollFade } from '../ui/useScrollFade';
import { TOOL_ICON } from './icons';
import { Thought } from './Thought';
import { Plan } from './Plan';
import { ReadGroup, ToolCall } from './ToolCall';
import { groupReadCalls } from './toolDetails';
import { Prose } from './Prose';
import { Permission } from './Permission';
import { QuestionRecord } from './Questions';
import { PlanDocument } from './PlanDocument';
import { TurnAttachments } from './Attachments';
import { elapsedLabel, splitCodexBlocks } from './folding';
import { rememberFold, rememberedFold } from './foldMemory';
import { ProcessHistory } from './ProcessHistory';
import { compactionForDisplay } from './compactionDisplay';
import { splitPlanSections } from './planSections';

// User message: color block / right-aligned bubble / plain text; ones Acpira sends automatically (/compact) render as a note line, not a bubble.
// Attachments (image thumbnails / file pills) sit above the text inside the same bubble.
// Clicking the card opens its inline editor, which also gives the full text for copying; no separate hover actions.
// Sticking within the exchange is the caller's job (`HistoryMessage` wraps it), so the editor can take the card's place without a layout jump;
// `compact` is its stuck state: the text folds to a few lines with a fading edge so a long prompt does not wall off the reply.
export function UserMessage({ turn, index, blobUrl, onEdit, compact }: { turn: UserTurn; index: number; blobUrl?: (blob: string) => string; onEdit?: () => void; compact?: boolean }) {
  const { userMessage } = useAppearance();
  const fade = useScrollFade<HTMLDivElement>();
  const text = useRef<HTMLDivElement>(null);
  const textRef = useMergedRefs(fade, text);
  useLayoutEffect(() => { if (compact && text.current) text.current.scrollTop = 0; }, [compact]);
  if (turn.auto) {
    return (
      <div className="enter px-pad" style={{ '--i': index } as CSSProperties}>
        <Row className="text-fg-3"><span>{t('turns.autoCompact')}</span></Row>
      </div>
    );
  }
  return (
    <div className={cn('flex w-full min-w-0 flex-col', userMessage === 'bubble' && 'self-end max-w-[88%]')}>
      <div
        onClick={onEdit ? e => {
          // Preserve text selection and attachment preview controls inside the card.
          if ((e.target as HTMLElement).closest('button, a, [role="dialog"]') || window.getSelection()?.toString()) return;
          onEdit();
        } : undefined}
        className={cn(
          'relative flex max-h-(--user-message-max) w-full shrink-0 flex-col gap-gap px-pad text-1 text-fg-1 [overflow-wrap:anywhere]',
          // Cards stick within an exchange; an opaque surface under the translucent chip color stops replies bleeding through.
          userMessage !== 'plain' && 'user-message-card rounded-lg py-gap bg-bg-0 bg-[linear-gradient(var(--chip),var(--chip))] shadow-[inset_0_0_0_1px_var(--conversation-line)] transition-shadow',
          // An editable prompt opens on click; its outline firms up while the actions appear below.
          onEdit && 'cursor-pointer',
          onEdit && userMessage !== 'plain' && 'hover:shadow-[inset_0_0_0_1px_var(--conversation-line-focus)]',
          userMessage === 'plain' && 'bg-bg-0 py-gap font-medium',
        )}
      >
        {turn.attachments?.length ? <TurnAttachments attachments={turn.attachments} blobUrl={blobUrl} /> : null}
        {turn.text && <div ref={textRef} className={cn(
          'scroll-fade scroll-thin min-h-0 whitespace-pre-wrap transition-[max-height] duration-(--dur-open) ease-out [--scroll-fade-size:var(--text-1-lh)] [overflow-anchor:none]',
          // The command mark's background overhangs its line box; without room inside the padding box the scrollport shaves its top / left / bottom edges.
          turn.command && turn.text.startsWith(`/${turn.command}`) && 'py-0.5 pl-1',
          // Folded text does not take the wheel: scrolling over a stuck card keeps moving the conversation.
          compact ? 'max-h-(--user-message-stuck-max) overflow-hidden' : 'max-h-(--user-message-max) overflow-y-auto',
        )}>{turn.command && turn.text.startsWith(`/${turn.command}`)
          ? <><mark className={COMMAND_MARK}>/{turn.command}</mark>{turn.text.slice(turn.command.length + 1)}</> : turn.text}</div>}
      </div>
    </div>
  );
}

type OnPermission = (blockId: string, optionId: string) => void;

// Agent message: consecutive "lines" (thought / plan / tool, commands included) are grouped together; prose / permission cards each stand alone as blocks.
// The top-level activity owns the only Orb; detailed rows show their own verbs with static icons.
// Memoized: the host pushes the whole view on every stream chunk and `reuse` keeps finished turns by reference, so only the live turn renders.
// `memoryKey` names the turn for fold memory (session + turn); without one the fold state lives only in the component.
export const AgentMessage = memo(function AgentMessage({ turn, index, running, onPermission, compacting, memoryKey }: { turn: AgentTurn; index: number; running: boolean; onPermission: OnPermission; compacting?: boolean; memoryKey?: string }) {
  if (compacting) turn = compactionForDisplay(turn, running);
  const sections = splitPlanSections(turn.blocks);
  return <RowEntranceContext.Provider value={running}><div className="flex min-w-0 flex-col gap-gap px-pad [--row:var(--chat-row)]">
    {sections.map((section, i) => {
      const last = i === sections.length - 1;
      // Only the continuation owns live activity and the turn outcome. Earlier
      // sections have no independent timing; repeating the full duration lies.
      const content = { ...turn, blocks: section.blocks,
        ...(!last ? { stop: undefined, error: undefined, command: undefined } : {}),
        ...(sections.length > 1 ? { startedAt: undefined, endedAt: undefined } : {}),
      };
      return <Fragment key={section.key}>
        {(section.blocks.length > 0 || (last && (running || outcomeOf(turn)))) && <AgentContent turn={content} index={index}
          running={last && running} onPermission={onPermission}
          memoryKey={memoryKey && (i === 0 ? memoryKey : `${memoryKey}:after-plan:${section.key}`)} />}
        {section.plan && <PlanDocument block={section.plan}
          permission={turn.blocks.find((b): b is PermissionBlock => b.type === 'permission' && b.planId === section.plan!.id)} onChoose={onPermission} />}
      </Fragment>;
    })}
  </div></RowEntranceContext.Provider>;
});

function AgentContent({ turn, index, running, onPermission, memoryKey }: { turn: AgentTurn; index: number; running: boolean; onPermission: OnPermission; memoryKey?: string }) {
  const { fold } = useAppearance();
  if (fold === 'codex') return <CodexMessage turn={turn} running={running} onPermission={onPermission} memoryKey={memoryKey} />;
  // Plan approvals live on the plan card and the open question card above the composer; neither takes a slot in the message
  const groups = groupBlocks(detailBlocks(turn, running).filter(b => (b.type !== 'permission' || !b.planId) && (b.type !== 'question' || !!b.outcome)));
  return (
    <div className="flex flex-col gap-gap">
      <Activity turn={turn} running={running} />
      {groups.map((g, gi) => (
        <div key={g.kind === 'block' && 'id' in g.block && g.block.id ? g.block.id : `g${gi}`}>
          {g.kind === 'lines'
            ? <Lines blocks={g.blocks} fold={fold} running={running} />
            : <Block block={g.block} onPermission={onPermission} />}
        </div>
      ))}
      {!running && outcomeOf(turn) && (
        <div>
          <Outcome turn={turn} />
        </div>
      )}
    </div>
  );
}

// How the turn ended, when that is worth a line: it stopped short (error / refusal / a limit / stopped by hand), or it ended normally with nothing to show.
// Nothing for a normal end with content, nor for turns persisted before `stop` existed
function outcomeOf(turn: AgentTurn): string | undefined {
  return turnOutcome(turn, getLocale());
}

// One faint row closing the message: a warning glyph for the short stops, none for "stopped" / "no reply"; the error's own words ride along as the target
function Outcome({ turn }: { turn: AgentTurn }) {
  const { toolLine } = useAppearance();
  const warn = turn.stop !== 'cancelled' && turn.stop !== 'end_turn';
  const lead = toolLine === 'text' || !warn ? undefined : <TriangleAlert className="size-icon" strokeWidth={1.5} />;
  return (
    <Row lead={lead} className="text-fg-3">
      <RowLabel>{outcomeOf(turn)}</RowLabel>
      {turn.stop === 'error' && turn.error?.message && <RowTarget className="text-fg-3">{turn.error.message}</RowTarget>}
    </Row>
  );
}

// Turn-level activity is independent of the latest tool and the fold's expansion state.
// A pending user decision suspends the animation until the turn can continue.
function compactionInActivity(turn: AgentTurn) {
  return !turn.blocks.some(b => b.type === 'permission' || (b.type === 'question' && !b.outcome))
    && turn.blocks.some(b => b.type === 'compaction' && b.status === 'in_progress');
}

// The activity owns live compaction; terminal statuses stay in transcript history.
function detailBlocks(turn: AgentTurn, running: boolean) {
  return running && compactionInActivity(turn)
    ? turn.blocks.filter(b => b.type !== 'compaction' || b.status !== 'in_progress')
    : turn.blocks;
}

function liveActivity(turn: AgentTurn) {
  if (turn.blocks.some(b => b.type === 'permission')) {
    return { label: t('host.awaitingApproval'), active: false, lead: <Hand className="size-icon" strokeWidth={1.5} /> };
  }
  if (turn.blocks.some(b => b.type === 'question' && !b.outcome)) {
    return { label: t('host.awaitingAnswers'), active: false, lead: <MessageCircleQuestion className="size-icon" strokeWidth={1.5} /> };
  }
  return { label: t(compactionInActivity(turn) ? 'turns.compacting' : 'host.working'), active: true, lead: <Orb kind="think" /> };
}

function Activity({ turn, running }: { turn: AgentTurn; running: boolean }) {
  const [present, setPresent] = useState(running);
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (running) { setPresent(true); return; }
    if (!present) return;
    // Retire the row only after its native exit transition finishes. Motion-off
    // produces no transitions and removes it immediately; no artificial delay is used.
    const animations = ref.current?.getAnimations() ?? [];
    if (!animations.length) { setPresent(false); return; }
    let disposed = false;
    void Promise.allSettled(animations.map(animation => animation.finished)).then(() => {
      if (!disposed) setPresent(false);
    });
    return () => { disposed = true; };
  }, [running, present]);
  const activity = liveActivity(turn);
  if (!running && !present) return null;
  return (
    // Fade the complete label before collapsing its slot; clipping the row at the
    // same time would cut the text away during the first few frames of the exit.
    // The negative closing margin removes the parent's gap together with the row.
    <div ref={ref} inert={!running} className={cn(
      'grid transition-[grid-template-rows,opacity,margin-bottom] duration-(--dur-open) ease-out',
      running ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0 -mb-gap [transition-delay:var(--dur-open),0s,var(--dur-open)]',
    )}>
      <div className="min-h-0 overflow-hidden">
        <Row lead={activity.lead} className="font-medium">
          <RowLabel shimmer={activity.active}>{activity.label}</RowLabel>
        </Row>
      </div>
    </div>
  );
}

const LINE_TYPES = new Set(['thought', 'plan', 'tool_call', 'compaction']);
type Group = { kind: 'lines'; blocks: AgentBlock[] } | { kind: 'block'; block: AgentBlock };

function groupBlocks(blocks: AgentBlock[]): Group[] {
  const out: Group[] = [];
  for (const b of blocks) {
    const last = out[out.length - 1];
    if (LINE_TYPES.has(b.type)) {
      if (last?.kind === 'lines') last.blocks.push(b);
      else out.push({ kind: 'lines', blocks: [b] });
    } else out.push({ kind: 'block', block: b });
  }
  return out;
}

// A group of lines in cursor mode: runs of finished read-only actions (read / search / fetch, ≥ 2) fold into one expandable row, everything else stays flat
function Lines({ blocks, fold, running }: { blocks: AgentBlock[]; fold: Appearance['fold']; running: boolean }) {
  const items = fold === 'cursor' ? foldReadOnly(blocks) : blocks.map(b => ({ kind: 'one' as const, block: b }));
  return (
    <div className="process-rows flex flex-col gap-0.5">
      {items.map((it, i) => it.kind === 'one'
        ? <LineBlock key={i} block={it.block} />
        : <CursorFold key={it.blocks[0]!.id} blocks={it.blocks} />)}
    </div>
  );
}

type LineItem = { kind: 'one'; block: AgentBlock } | { kind: 'fold'; blocks: ToolCallBlock[] };

const READ_ONLY: ReadonlySet<ToolKind> = new Set<ToolKind>(['read', 'search', 'fetch']);

function isFoldableRead(b: AgentBlock): b is ToolCallBlock {
  return b.type === 'tool_call' && READ_ONLY.has(b.kind) && b.status !== 'in_progress' && b.status !== 'pending';
}

function foldReadOnly(blocks: AgentBlock[]): LineItem[] {
  const out: LineItem[] = [];
  let run: ToolCallBlock[] = [];
  const flush = () => {
    if (run.length >= 2) out.push({ kind: 'fold', blocks: run });
    else for (const b of run) out.push({ kind: 'one', block: b });
    run = [];
  };
  for (const b of blocks) {
    if (isFoldableRead(b)) run.push(b);
    else { flush(); out.push({ kind: 'one', block: b }); }
  }
  flush();
  return out;
}

// Disclosure shared by fold rows: lead-slot rules match other rows (toolLine=text has no slot), the trailing chevron rotates 90° when open;
// the body isn't indented — expanded rows align vertically with the head row, and open/close alone marks the hierarchy
function FoldRow({ icon, children, body, open, onToggle }: { icon: ReactNode; children: ReactNode; body: ReactNode; open?: boolean; onToggle?: (open: boolean) => void }) {
  const { toolLine } = useAppearance();
  const [innerOpen, setInnerOpen] = useState(false);
  const expanded = open ?? innerOpen;
  return (
    <Disclosure tone="action" className="action-details"
      lead={toolLine === 'text' ? undefined : icon}
      indent={false}
      open={expanded}
      onToggle={next => { setInnerOpen(next); onToggle?.(next); }}
      trailing={<ChevronRight className="size-3 transition-transform group-data-[open]:rotate-90" strokeWidth={1.75} />}
      body={<ProcessHistory>{body}</ProcessHistory>}
    >
      {children}
    </Disclosure>
  );
}

// Cursor mode: all read → "read N files", all search → "searched N times", mixed → "explored N places"
function CursorFold({ blocks }: { blocks: ToolCallBlock[] }) {
  const kinds = new Set(blocks.map(b => b.kind));
  const only = kinds.size === 1 ? blocks[0]!.kind : undefined;
  const files = new Set(blocks.map(b => b.target).filter(Boolean)).size || blocks.length;
  const label = only === 'read' ? t('turns.readFiles', { n: files }) : only === 'search' ? t('turns.searched', { n: blocks.length }) : t('turns.explored', { n: blocks.length });
  const Icon = only ? TOOL_ICON[only] : Compass;
  return (
    <FoldRow icon={<Icon className="size-icon" strokeWidth={1.5} />} body={<ProcessBlocks blocks={blocks} />}>
      <span>{label}</span>
    </FoldRow>
  );
}

// One process area per turn. Thoughts keep their normal disclosure under the activity row until the
// first tool call makes the area foldable; the same panel stays mounted across that change so a row
// the user opened is never rebuilt. Permission cards stay outside; the latest reply remains visible while it streams.
function CodexMessage({ turn, running, onPermission, memoryKey }: { turn: AgentTurn; running: boolean; onPermission: OnPermission; memoryKey?: string }) {
  const { process, reply, permissions } = splitCodexBlocks(detailBlocks(turn, running));
  const foldable = turn.blocks.some(block => block.type === 'tool_call');
  return (
    <div className="flex flex-col gap-gap">
      {!foldable && <Activity turn={turn} running={running} />}
      <CodexFold turn={turn} blocks={process} running={running} foldable={foldable} memoryKey={memoryKey} />
      {reply.map((block, i) => <Prose key={i} block={block} />)}
      {permissions.filter(block => !block.planId).map(block => <Permission key={block.id} block={block} onChoose={id => onPermission(block.id, id)} />)}
      {!running && outcomeOf(turn) && <Outcome turn={turn} />}
    </div>
  );
}

// A turn first opened with tools starts collapsed. During streaming, content already shown before the first
// tool stays visible when the process becomes foldable. Afterwards only a manual toggle moves it.
// The toggle is also written to fold memory under `memoryKey`, so a rebuilt message (or a reloaded webview) reopens
// what the reader had opened instead of snapping shut mid-turn.
function CodexFold({ turn, blocks, running, foldable, memoryKey }: { turn: AgentTurn; blocks: AgentBlock[]; running: boolean; foldable: boolean; memoryKey?: string }) {
  const recall = (key: string | undefined) => ({ key, manual: key ? rememberedFold(key) : undefined });
  const [choice, setChoice] = useState(() => recall(memoryKey));
  // A key that changes under a mounted fold (a turn that gains its start time) re-reads memory instead of keeping a stranger's choice
  if (choice.key !== memoryKey) setChoice(recall(memoryKey));
  const manual = choice.key === memoryKey ? choice.manual : undefined;
  const openedInside = useRef(0);
  const visibleBeforeTools = useRef(false);
  // Remember committed content, including commentary that moves from the reply into the process on the
  // first tool call. The pre-tool Working row is not a disclosure, so it cannot record a manual choice.
  useLayoutEffect(() => {
    if (!foldable) visibleBeforeTools.current = blocks.length > 0
      || turn.blocks.some(block => block.type === 'text' && !!block.markdown.trim());
  }, [foldable, blocks, turn.blocks]);
  const latched = useRef<boolean | undefined>(undefined);
  if (!foldable) latched.current = undefined;
  else latched.current ??= visibleBeforeTools.current || openedInside.current > 0;
  const observe = useCallback((next: boolean) => { openedInside.current += next ? 1 : -1; }, []);
  const toggle = useCallback((next: boolean) => { setChoice({ key: memoryKey, manual: next }); if (memoryKey) rememberFold(memoryKey, next); }, [memoryKey]);
  const open = !foldable || (manual ?? latched.current);
  const activity = liveActivity(turn);
  const CompletionIcon = turn.stop === 'cancelled' ? X : outcomeOf(turn) ? TriangleAlert : Check;
  const lead = running ? activity.lead : <CompletionIcon className="size-icon" strokeWidth={1.5} />;
  const label = running ? activity.label : outcomeOf(turn) ?? t('turns.done');
  const elapsed = !running && turn.startedAt !== undefined && turn.endedAt !== undefined ? elapsedLabel(turn) : undefined;
  if (!foldable && blocks.length === 0) return null;
  return (
    <Collapsible.Root open={open} onOpenChange={toggle} className="group flex min-w-0 flex-col" data-open={open || undefined}>
      {foldable && (
        <Collapsible.Trigger render={<Row as="button" interactive lead={lead} title={label} />}>
          <RowLabel shimmer={running && activity.active}>{label}</RowLabel>
          {elapsed && <span className="min-w-0 truncate text-fg-3" title={elapsed}>{elapsed}</span>}
          <ChevronRight className={cn('size-3 shrink-0 self-center transition-transform', open && 'rotate-90')} strokeWidth={1.75} />
        </Collapsible.Trigger>
      )}
      {blocks.length > 0 && (
        // Nested rows extend their hit area beyond the text column; reserve it inside the clip so its edges cannot cut off row corners.
        <Collapsible.Panel className="-mx-hit [&>div]:px-hit">
          <div className={cn(foldable && 'pt-1 pb-1.5')}>
            <DisclosureObserverContext.Provider value={observe}>
              <ProcessHistory><ProcessBlocks blocks={blocks} /></ProcessHistory>
            </DisclosureObserverContext.Provider>
          </div>
        </Collapsible.Panel>
      )}
    </Collapsible.Root>
  );
}

// Process details retain static icons; only the currently running verb shimmers.
// Unkeyed blocks take their transcript position, so a read group forming ahead of them does not remount them.
function ProcessBlocks({ blocks }: { blocks: AgentBlock[] }) {
  let position = 0;
  return groupReadCalls(blocks).map(item => {
    const at = position;
    position += Array.isArray(item) ? item.length : 1;
    return Array.isArray(item)
      ? <ReadGroup key={item[0]!.id} blocks={item} />
      : item.type === 'tool_call' ? <ToolCall key={item.id} block={item} grouped />
      : item.type === 'text' ? <Prose key={at} block={item} />
      : <LineBlock key={'id' in item ? item.id : at} block={item} />;
  });
}

function LineBlock({ block }: { block: AgentBlock }) {
  if (block.type === 'thought') return <Thought block={block} />;
  if (block.type === 'plan') return <Plan block={block} />;
  if (block.type === 'tool_call') return <ToolCall block={block} />;
  if (block.type === 'compaction') return <Compaction block={block} />;
  // The open card is pinned above the composer by the shell; only a resolved one has a place in the message
  if (block.type === 'question') return block.outcome ? <QuestionRecord block={block} /> : null;
  return null;
}

// Context compaction is a localized status aligned with ordinary reply text.
function Compaction({ block }: { block: CompactionBlock }) {
  const running = block.status === 'in_progress';
  const label = running ? t('turns.compacting') : block.status === 'completed' ? t('turns.compacted') : block.status === 'failed' ? t('turns.compactFailed') : t('turns.compactCancelled');
  return (
    <Row className="text-fg-3">
      <Shimmer active={running}>{label}</Shimmer>
    </Row>
  );
}

function Block({ block, onPermission }: { block: AgentBlock; onPermission: OnPermission }) {
  if (block.type === 'text') return <Prose block={block} />;
  if (block.type === 'permission') return block.planId ? null : <Permission block={block} onChoose={id => onPermission(block.id, id)} />;
  return <LineBlock block={block} />;
}
