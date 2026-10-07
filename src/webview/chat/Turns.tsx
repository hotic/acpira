import { Fragment, createContext, memo, useCallback, useContext, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import { Bot, Check, ChevronRight, Compass, Hand, MessageCircleQuestion, Shrink, TriangleAlert, X } from 'lucide-react';
import type { AgentBlock, AgentTurn, CompactionBlock, FailureAction, NoticeBlock, PermissionBlock, SlashCommand, SteerBlock, ToolCallBlock, ToolKind, TurnSettings, UserTurn } from '@shared/transcript';
import type { SubagentSummary } from '@shared/subagents';
import { isImageResultBlock } from '@shared/imageTools';
import { useAppearance, type Appearance } from '../appearance';
import { getLocale, t } from '../i18n';
import { commandSegments } from './PromptInput';
import { promptMarks } from './slashCommands';
import { absorbedNotices, hasTurnContent, turnOutcome } from './turnOutcome';
import { EntranceOnce, Row, RowLabel, RowTarget, RowEntranceContext, EntranceScopeContext } from '../ui/Row';
import { Button } from '../ui/Button';
import { Disclosure } from '../ui/Disclosure';
import { Collapsible, LazyPanelContext } from '../ui/Collapsible';
import { Orb } from '../effects/Orb';
import { cn } from '../ui/cn';
import { useScrollFade } from '../ui/useScrollFade';
import { TOOL_ICON } from './icons';
import { Thought } from './Thought';
import { Plan } from './Plan';
import { ToolCall } from './ToolCall';
import { ToolGroup } from './ToolGroup';
import { foldStates, groupProcess, itemEntrance, type ProcessItem } from './processGroups';
import { AutoFoldContext, AutoFoldItem, AutoFoldStore } from './autoFold';
import { useFollowing } from './useBottomFollow';
import { Prose } from './Prose';
import { AgentImage } from './AgentImage';
import { GeneratedImages } from './GeneratedImage';
import { Permission } from './Permission';
import { QuestionRecord } from './Questions';
import { BlobUrlContext } from './fileLinks';
import { PlanDocument } from './PlanDocument';
import { TurnAttachments } from './Attachments';
import { elapsedLabel, splitCodexBlocks } from './folding';
import { rememberFold, rememberedFold } from './foldMemory';
import { ProcessHistory } from './ProcessHistory';
import { compactionForDisplay } from './compactionDisplay';
import { splitPlanSections } from './planSections';
import { TurnActions } from './TurnActions';
import { SubagentGroup } from './subagents/SubagentGroup';
import { breadcrumb, delegatedIds, nodesByTurn, placeNodes, subagentTitle } from './subagents/subagentState';
import { Surface, surfaceVariants } from '../ui/Surface';

// User message: color block / right-aligned bubble / plain text; ones Acpira sends automatically (/compact) render nothing,
// since the reply's compaction row already says what happened.
// Attachments (image thumbnails / file pills) sit above the text inside the same bubble.
// Clicking the card opens its inline editor, which also gives the full text for copying; no separate hover actions.
// Sticking within the exchange is the caller's job (`HistoryMessage` wraps it), so the editor can take the card's place without a layout jump;
// The height cap and inner scrolling stay identical before and after sticking, preserving the reading position.
// Trailing blank lines are not displayed; the turn keeps its original text.
export function UserMessage({ turn, blobUrl, onEdit, commands, summons }: {
  turn: UserTurn; index: number; blobUrl?: (blob: string) => string; onEdit?: () => void; commands?: readonly SlashCommand[];
  // Names of the enabled subagent personas: their `@name` tokens get the summon pill
  summons?: readonly string[];
}) {
  const { userMessage } = useAppearance();
  const fade = useScrollFade<HTMLDivElement>();
  if (turn.auto) return null;
  // The same marks the composer painted while this was being typed; a recorded command keeps its pill
  // even after the agent stops advertising it
  const shown = turn.text.trimEnd();
  const marks = promptMarks(commands ?? [], summons ?? [], shown);
  if (turn.command && shown.startsWith(`/${turn.command}`) && marks[0]?.start !== 0)
    marks.unshift({ start: 0, name: turn.command });
  return (
    <div className={cn('flex w-full min-w-0 flex-col', userMessage === 'bubble' && 'self-end max-w-[88%]')}>
      <Surface
        tone={userMessage !== 'plain' ? 'message' : undefined}
        onClick={onEdit ? e => {
          // Preserve text selection and attachment preview controls inside the card.
          if ((e.target as HTMLElement).closest('button, a, [role="dialog"]') || window.getSelection()?.toString()) return;
          onEdit();
        } : undefined}
        className={cn(
          'relative flex w-full shrink-0 flex-col px-pad text-1 text-fg-1 [overflow-wrap:anywhere]',
          // Cards stick within an exchange; an opaque surface under the translucent chip color stops replies bleeding through.
          userMessage !== 'plain' && 'user-message-card py-gap bg-[linear-gradient(var(--chip),var(--chip))] transition-shadow',
          // An editable prompt opens on click; its outline firms up while the actions appear below.
          onEdit && 'cursor-pointer',
          onEdit && userMessage !== 'plain' && 'hover:shadow-[inset_0_0_0_1px_var(--conversation-line-focus)]',
          userMessage === 'plain' && 'bg-bg-0 py-gap font-medium',
        )}
      >
        {/* Attachments and text share one scrollport, so thumbnails scroll away instead of holding a fixed share of the cap */}
        <div ref={fade} className="scroll-fade scroll-thin flex max-h-(--user-message-max) min-h-0 flex-col gap-gap overflow-y-auto [--scroll-fade-size:var(--text-1-lh)] [overflow-anchor:none]">
          {turn.attachments?.length ? <TurnAttachments attachments={turn.attachments} blobUrl={blobUrl} /> : null}
          {shown && <div className={cn(
            'shrink-0 whitespace-pre-wrap',
            // A command / summon mark's background overhangs its line box on any side; without room inside the padding box the scrollport shaves it.
            marks.length > 0 && 'py-0.5 px-1',
          )}>{marks.length ? commandSegments(shown, marks) : shown}</div>}
        </div>
      </Surface>
    </div>
  );
}

// A prompt the user steered into the running turn: the same card as a sent message, inside the turn's process at the point
// it joined the loop (the turn is still one run, so nothing before it reads as finished). The fold's body is split around
// the card and only the parts between fold, so a closed fold never hides what the user said and the card slides under
// the fold head with the closing parts instead of being drawn a second time outside them.
// It is not an exchange of its own, so it neither sticks nor opens the history editor.
// Inline and lifted it has the same width as the original prompt card, so opening or closing the fold never resizes it
function SteeredMessage({ block }: { block: SteerBlock }) {
  const blobUrl = useContext(BlobUrlContext);
  const turn = useMemo<UserTurn>(() => ({ role: 'user', text: block.text, ...(block.attachments ? { attachments: block.attachments } : {}) }), [block]);
  // A steered prompt belongs to the current exchange, but its card is still a user message.
  // Cancel the agent content inset so it shares the same edges as the original prompt card.
  return <div title={t('turns.steered')} className="steered-message -mx-pad flex min-w-0 flex-col"><UserMessage turn={turn} index={0} blobUrl={blobUrl} /></div>;
}

type OnPermission = (blockId: string, optionId: string) => void;

// AIR sessionFailure notices: the turn scopes which rows may offer their actions (the last settled turn,
// never while streaming) and which notice the turn's own error card already explains — that one is
// suppressed here instead of reporting the same failure twice
interface NoticeActions {
  onAction?: (action: FailureAction) => void;
  showActions?: boolean;
  suppressId?: string;
  // Error notices folded into the turn's outcome row (`absorbedNotices`); they render there, not as rows of their own
  absorbed?: NoticeBlock[];
}
const NoticeActionContext = createContext<NoticeActions | undefined>(undefined);

// Lines under a row with a lead start at the label column, past the lead slot and its gap
const UNDER_LABEL = 'pl-indent';

// One row per failure id: an error marks only its glyph with the warn tone and keeps the title in body grey,
// a warning stays in the transcript's quiet color. Details wrap under the title at the label column; the
// adapter's own actions render as small secondary buttons, in payload order
function NoticeRow({ block }: { block: NoticeBlock }) {
  const ctx = useContext(NoticeActionContext);
  if (block.id === ctx?.suppressId || ctx?.absorbed?.includes(block)) return null;
  const error = block.severity === 'error';
  const buttons = error && ctx?.showActions && ctx.onAction ? block.actions : [];
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <Row lead={<TriangleAlert className={cn('size-icon', error && 'text-warn')} strokeWidth={1.5} />} className={error ? 'text-fg-2' : 'text-fg-3'}>
        <RowLabel className="whitespace-pre-wrap">{block.title}</RowLabel>
      </Row>
      {block.details && (
        <p className={cn('m-0 min-w-0 whitespace-pre-wrap text-3 text-fg-3 [overflow-wrap:anywhere]', UNDER_LABEL)}>{block.details}</p>
      )}
      {buttons.length > 0 && (
        <div className={cn('flex flex-wrap gap-gap pt-1', UNDER_LABEL)}>
          {buttons.map(a => (
            <Button key={a} variant="secondary" className="h-ctl-sm px-2 text-3" onClick={() => ctx?.onAction?.(a)}>
              {a === 'retry' ? t('common.retry') : a === 'new_session' ? t('notice.continueNew') : t('notice.goLogin')}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}

// Agent message: consecutive "lines" (thought / plan / tool, commands included) are grouped together; prose / permission cards each stand alone as blocks.
// The top-level activity owns the only Orb; detailed rows show their own verbs with static icons.
// Memoized: the host pushes the whole view on every stream chunk and `reuse` keeps finished turns by reference, so only the live turn renders.
// `memoryKey` names the turn for fold memory (session + turn); without one the fold state lives only in the component.
export const AgentMessage = memo(function AgentMessage({ turn, index, running, onPermission, compacting, memoryKey, turnIndex, last, settings, subagents, allSubagents, onInspect, actions = true, lead = 'orb', onFailureAction, joined }: {
  turn: AgentTurn; index: number; running: boolean; onPermission: OnPermission; compacting?: boolean; memoryKey?: string; turnIndex: number; last: boolean; settings?: TurnSettings;
  // Nodes anchored to this turn plus the session-wide list (breadcrumbs/descendant counts may cross turns)
  subagents?: SubagentSummary[]; allSubagents?: SubagentSummary[]; onInspect?: (id: string) => void;
  // The subagent inspector renders turns without the copy/fork/stats row
  actions?: boolean;
  // Working-row lead: the Orb belongs to the root conversation; the inspector uses a static icon
  lead?: 'orb' | 'static';
  // AIR sessionFailure notice actions (retry / sign in / new session), available on the last settled turn
  onFailureAction?: (action: FailureAction) => void;
  // Continues the turn above (the hidden continue after an account switch): one row gap instead of a message gap
  joined?: boolean;
}) {
  const absorbed = useMemo(() => absorbedNotices(turn), [turn]);
  const base = compacting ? compactionForDisplay(turn, running) : turn;
  // Absorbed notices leave the block list, so an empty line group cannot leave a stray gap behind
  const raw = useMemo(() => absorbed.length ? { ...base, blocks: base.blocks.filter(b => b.type !== 'notice' || !absorbed.includes(b)) } : base, [base, absorbed]);
  // Everything but the section split reads the turn without its delegation rows
  const shown = useMemo(() => raw.blocks.some(b => b.type === 'tool_call' && b.subagentId !== undefined)
    ? { ...raw, blocks: raw.blocks.filter(b => b.type !== 'tool_call' || b.subagentId === undefined) } : raw, [raw]);
  // Subagent rows sit where they were delegated: a delegation row (tool call with `subagentId`) splits the turn
  // like a plan does, and its node plus that node's same-turn descendants render at the split. Nodes with no
  // delegation row in this turn trail the content as before. Delegation rows themselves never render.
  const placed = useMemo(() => subagents?.length && onInspect ? delegatedIds(raw.blocks, subagents) : undefined, [raw.blocks, subagents, onInspect]);
  const { byId: placedNodes, rest: restNodes } = useMemo(() => placed?.size ? placeNodes(subagents!, placed) : { byId: undefined, rest: subagents }, [placed, subagents]);
  const sections = useMemo(() => splitPlanSections(raw.blocks, placed), [raw.blocks, placed]);
  // A /compact reply that so far holds only its compaction row: that row shimmers in place, so no generic Working row above it
  const compactionOnly = !!compacting && shown.blocks.length > 0 && shown.blocks.every(b => b.type === 'compaction');
  const entranceScope = useId();
  const entrance = useMemo(() => ({ live: running, scope: memoryKey ?? entranceScope }), [running, memoryKey, entranceScope]);
  const noticeActions = useMemo<NoticeActions>(() => ({
    onAction: onFailureAction,
    showActions: last && !running,
    ...(turn.error?.failureId !== undefined ? { suppressId: turn.error.failureId } : {}),
    ...(absorbed.length ? { absorbed } : {}),
  }), [onFailureAction, last, running, turn.error?.failureId, absorbed]);
  // A settled turn mounts fold bodies on first open; a live one keeps them mounted so streamed content stays in step while closed
  return <LazyPanelContext.Provider value={!running}><EntranceScopeContext.Provider value={entrance}><RowEntranceContext.Provider value={running}><NoticeActionContext.Provider value={noticeActions}><div className={cn('group/turn flex min-w-0 flex-col gap-gap px-pad [--row:var(--chat-row)]', joined && '-mt-msg-join')}>
    {sections.map((section, i) => {
      const lastSection = i === sections.length - 1;
      // Only the continuation owns live activity and the turn outcome. Earlier
      // sections have no independent timing; repeating the full duration lies.
      const content = { ...shown, blocks: section.blocks,
        ...(!lastSection ? { stop: undefined, error: undefined, command: undefined } : {}),
        ...(sections.length > 1 ? { startedAt: undefined, endedAt: undefined } : {}),
      };
      return <Fragment key={section.key}>
        {(section.blocks.length > 0 || (lastSection && (running || outcomeOf(shown)))) && <AgentContent turn={content} index={index}
          running={lastSection && running} compactionOnly={compactionOnly} onPermission={onPermission}
          memoryKey={memoryKey && (i === 0 ? memoryKey : `${memoryKey}:after-plan:${section.key}`)}
          subagents={lastSection ? restNodes : undefined} allSubagents={lastSection ? allSubagents : undefined} onInspect={onInspect} lead={lead} />}
        {section.subagents && placedNodes && onInspect && <PlacedSubagents nodes={section.subagents.flatMap(id => placedNodes.get(id) ?? [])}
          all={allSubagents ?? subagents!} onInspect={onInspect} onPermission={onPermission} />}
        {section.plan && <PlanDocument block={section.plan}
          permission={shown.blocks.find((b): b is PermissionBlock => b.type === 'permission' && b.planId === section.plan!.id)} onChoose={onPermission} />}
      </Fragment>;
    })}
    {actions && !running && !compacting && hasTurnContent(turn) && <TurnActions turn={turn} turnIndex={turnIndex} last={last} settings={settings} />}
  </div></NoticeActionContext.Provider></RowEntranceContext.Provider></EntranceScopeContext.Provider></LazyPanelContext.Provider>;
});

interface SubagentSlots {
  subagents?: SubagentSummary[];
  allSubagents?: SubagentSummary[];
  onInspect?: (id: string) => void;
  lead?: 'orb' | 'static';
}

function AgentContent({ turn, index, running, compactionOnly, onPermission, memoryKey, subagents, allSubagents, onInspect, lead }: { turn: AgentTurn; index: number; running: boolean; compactionOnly?: boolean; onPermission: OnPermission; memoryKey?: string } & SubagentSlots) {
  const { fold } = useAppearance();
  const working = running && !compactionOnly;
  if (fold === 'codex') return <CodexMessage turn={turn} running={running} working={working} onPermission={onPermission} memoryKey={memoryKey} subagents={subagents} allSubagents={allSubagents} onInspect={onInspect} lead={lead} />;
  // Plan approvals live on the plan card and the open question card above the composer; neither takes a slot in the message
  const groups = groupBlocks(turn.blocks.filter(b => (b.type !== 'permission' || !b.planId) && (b.type !== 'question' || !!b.outcome)));
  return (
    <div className="flex flex-col gap-gap">
      <Activity turn={turn} running={working} leadKind={lead} />
      {groups.map((g, gi) => (
        <div key={g.kind === 'block' && 'id' in g.block && g.block.id ? g.block.id : `g${gi}`}>
          {g.kind === 'lines'
            ? <Lines blocks={g.blocks} fold={fold} running={running} />
            : <Block block={g.block} onPermission={onPermission} />}
        </div>
      ))}
      {subagents !== undefined && subagents.length > 0 && onInspect !== undefined && (
        <>
          <SubagentGroup nodes={subagents} all={allSubagents ?? subagents} onInspect={onInspect} />
          <ChildPermissions nodes={subagents} all={allSubagents ?? subagents} onPermission={onPermission} />
        </>
      )}
      {!running && outcomeOf(turn) && (
        <div>
          <Outcome turn={turn} />
        </div>
      )}
    </div>
  );
}

// Subagent rows at their delegation point, with any pending child approvals right under them
function PlacedSubagents({ nodes, all, onInspect, onPermission }: { nodes: SubagentSummary[]; all: SubagentSummary[]; onInspect: (id: string) => void; onPermission: OnPermission }) {
  if (!nodes.length) return null;
  return <div className="flex flex-col gap-gap">
    <SubagentGroup nodes={nodes} all={all} onInspect={onInspect} />
    <ChildPermissions nodes={nodes} all={all} onPermission={onPermission} />
  </div>;
}

// A summoned child's approval sits right under its row, its edge at the row's label: the row already names who asks,
// so the card's title block and provenance line would be mostly empty space. Choices, deny-by-default and the
// command text are the full card's
function SummonedApproval({ block, onChoose }: { block: PermissionBlock; onChoose: (optionId: string) => void }) {
  return (
    <div className="ml-[calc(var(--spacing-lead)+var(--spacing-gap))] min-w-0">
      <Permission compact block={block} onChoose={onChoose} />
    </div>
  );
}

// A child's pending approval shows in the turn that delegated it, labeled with the chain it came from
function ChildPermissions({ nodes, all, onPermission }: { nodes: SubagentSummary[]; all: SubagentSummary[]; onPermission: OnPermission }) {
  const cards = nodes.flatMap(n => (n.permissions ?? []).map(b => ({ n, b })));
  if (!cards.length) return null;
  return (
    <>
      {cards.map(({ n, b }) => n.harness && !b.planId
        ? <SummonedApproval key={b.id} block={b} onChoose={id => onPermission(b.id, id)} />
        : (
        <Fragment key={b.id}>
          <Row dense className="text-3 text-fg-3">
            <span>{t('subagents.provenance', { path: breadcrumb(n.id, all).map(x => subagentTitle(x, t)).join(' › ') })}</span>
          </Row>
          <Permission block={b} onChoose={id => onPermission(b.id, id)} />
        </Fragment>
      ))}
    </>
  );
}

// How the turn ended, when that is worth a line: it stopped short (error / refusal / a limit / stopped by hand), or it ended normally with nothing to show.
// Nothing for a normal end with content, nor for turns persisted before `stop` existed
function outcomeOf(turn: AgentTurn): string | undefined {
  return turnOutcome(turn, getLocale());
}

// One faint row closing the message: a warning glyph for the short stops, none for "stopped" / "no reply"; the error's own words ride along as the target.
// Error notices absorbed into the outcome follow as quiet detail lines at the label column
function Outcome({ turn }: { turn: AgentTurn }) {
  const { toolLine } = useAppearance();
  const absorbed = useContext(NoticeActionContext)?.absorbed;
  const warn = turn.stop !== 'cancelled' && turn.stop !== 'end_turn';
  const lead = toolLine === 'text' || !warn ? undefined : <TriangleAlert className="size-icon" strokeWidth={1.5} />;
  const row = (
    <Row lead={lead} className="text-fg-3">
      <RowLabel>{outcomeOf(turn)}</RowLabel>
      {turn.stop === 'error' && turn.error?.message && <RowTarget className="text-fg-3">{turn.error.message}</RowTarget>}
    </Row>
  );
  if (!absorbed?.length) return row;
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      {row}
      {absorbed.map(n => (
        <p key={n.id} className={cn('m-0 min-w-0 whitespace-pre-wrap text-3 text-fg-3 [overflow-wrap:anywhere]', lead && UNDER_LABEL)}>
          {n.details ? `${n.title}\n${n.details}` : n.title}
        </p>
      ))}
    </div>
  );
}

// Turn-level activity is independent of the latest tool and the fold's expansion state.
// A pending user decision suspends the animation until the turn can continue.
// Live compaction is a row at its transcript position, like a running tool; the activity stays generic
// (a heading label read as belonging to an earlier, finished compaction row further up).
function liveActivity(turn: AgentTurn, leadKind: 'orb' | 'static' = 'orb') {
  if (turn.blocks.some(b => b.type === 'permission')) {
    return { label: t('host.awaitingApproval'), active: false, lead: <Hand className="size-icon" strokeWidth={1.5} /> };
  }
  if (turn.blocks.some(b => b.type === 'question' && !b.outcome)) {
    return { label: t('host.awaitingAnswers'), active: false, lead: <MessageCircleQuestion className="size-icon" strokeWidth={1.5} /> };
  }
  // The Orb belongs to the root turn only; observed child transcripts get a static lead
  const lead = leadKind === 'static' ? <Bot className="size-icon" strokeWidth={1.5} /> : <Orb kind="think" />;
  // A provider retry (network blip, overload) only relabels the working row; the adapter's wording stays in the tooltip
  const retry = turn.retry;
  if (retry) {
    const label = retry.attempt !== undefined && retry.max !== undefined
      ? t('turns.retryingAttempt', { attempt: retry.attempt, max: retry.max }) : t('turns.retrying');
    return { label, title: retry.detail, active: true, lead };
  }
  return { label: t('host.working'), active: true, lead };
}

function Activity({ turn, running, leadKind = 'orb' }: { turn: AgentTurn; running: boolean; leadKind?: 'orb' | 'static' }) {
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
  const activity = liveActivity(turn, leadKind);
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
        <Row lead={activity.lead} className="font-medium" title={activity.title}>
          <RowLabel shimmer={activity.active}>{activity.label}</RowLabel>
        </Row>
      </div>
    </div>
  );
}

const LINE_TYPES = new Set(['thought', 'plan', 'tool_call', 'compaction', 'notice']);
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

// One process area per turn, a fold from the first moment: its head carries the Orb and the Working label, and the
// thoughts before any tool call are its children, open by default (a separate Working row above a sibling Thinking
// row read as the same status twice, and swapping it for the fold head at the first tool replayed the entrance).
// Permission cards stay outside; the latest reply remains visible while it streams.
// `working` is false while a /compact reply shows only its compaction row, which retires the head the same way a settled turn does
function CodexMessage({ turn, running, working = running, onPermission, memoryKey, subagents, allSubagents, onInspect, lead }: { turn: AgentTurn; running: boolean; working?: boolean; onPermission: OnPermission; memoryKey?: string } & SubagentSlots) {
  const { process, reply, permissions, notices } = splitCodexBlocks(turn.blocks);
  const hasTools = turn.blocks.some(block => block.type === 'tool_call');
  // Generated and shown images are results, not process detail: they stay visible however the fold is set
  const generations = turn.blocks.filter(isImageResultBlock);
  // The process folds away only once the reply has finished drawing, not when its last chunk arrived
  const [replyBusy, setReplyBusy] = useState(false);
  return (
    <div className="flex flex-col gap-gap">
      <CodexFold turn={turn} blocks={process} running={working} replyBusy={replyBusy && reply.length > 0} hasTools={hasTools} memoryKey={memoryKey} lead={lead} />
      {notices.map(b => <NoticeRow key={b.id} block={b} />)}
      {generations.map(b => <GeneratedImages key={b.id} block={b} />)}
      {subagents !== undefined && subagents.length > 0 && onInspect !== undefined && (
        <>
          <SubagentGroup nodes={subagents} all={allSubagents ?? subagents} onInspect={onInspect} />
          <ChildPermissions nodes={subagents} all={allSubagents ?? subagents} onPermission={onPermission} />
        </>
      )}
      {reply.map((block, i) => <Prose key={i} block={block} onBusy={i === reply.length - 1 ? setReplyBusy : undefined} />)}
      {permissions.filter(block => !block.planId).map(block => <Permission key={block.id} block={block} onChoose={id => onPermission(block.id, id)} />)}
      {!running && outcomeOf(turn) && <Outcome turn={turn} />}
    </div>
  );
}

// The process fold follows the run: open while the turn works and while its reply is still drawing, closed once
// both are done, however the turn was mounted (a live session opened mid-turn shows what it is doing). A reader
// scrolled away from the bottom holds every automatic close until they return. A manual toggle always wins; it is
// written to fold memory under `memoryKey`, so a rebuilt message (or a reloaded webview) keeps what the reader chose.
function CodexFold({ turn, blocks, running, replyBusy, hasTools, memoryKey, lead }: { turn: AgentTurn; blocks: AgentBlock[]; running: boolean; replyBusy: boolean; hasTools: boolean; memoryKey?: string; lead?: 'orb' | 'static' }) {
  const recall = (key: string | undefined) => ({ key, manual: key ? rememberedFold(key) : undefined });
  const [choice, setChoice] = useState(() => recall(memoryKey));
  // A key that changes under a mounted fold (a turn that gains its start time) re-reads memory instead of keeping a stranger's choice
  if (choice.key !== memoryKey) setChoice(recall(memoryKey));
  const manual = choice.key === memoryKey ? choice.manual : undefined;
  const toggle = useCallback((next: boolean) => { setChoice({ key: memoryKey, manual: next }); if (memoryKey) rememberFold(memoryKey, next); }, [memoryKey]);
  const following = useFollowing();
  const held = useRef(false);
  const auto = running || replyBusy || (!following && held.current);
  held.current = auto;
  const items = useMemo(() => groupProcess(blocks), [blocks]);
  const store = useProcessFolds(items, manual === undefined, auto, following, memoryKey);
  // A turn that ends with no process worth a head (a plain reply, or only a compaction status) retires the head row
  // the way the Working row used to: fade, then collapse. A compaction status stays as a flat line.
  const retired = !running && !hasTools && !blocks.some(b => b.type !== 'compaction');
  const open = retired || (manual ?? auto);
  const activity = liveActivity(turn, lead);
  const CompletionIcon = turn.stop === 'cancelled' ? X : outcomeOf(turn) ? TriangleAlert : Check;
  const leadIcon = running ? activity.lead : <CompletionIcon className="size-icon" strokeWidth={1.5} />;
  const label = running ? activity.label : outcomeOf(turn) ?? t('turns.done');
  const elapsed = !running && turn.startedAt !== undefined && turn.endedAt !== undefined ? elapsedLabel(turn) : undefined;
  // Steered prompts stay visible between the folding parts of the process
  const parts = useMemo(() => splitAtSteers(items), [items]);
  const mounted = useRef(!retired);
  if (!retired) mounted.current = true;
  if (!mounted.current && blocks.length === 0) return null;
  const head = (
    <Collapsible.Trigger render={<Row as="button" interactive lead={leadIcon} title={(running && activity.title) || label} className={running ? surfaceVariants({ tone: 'status' }) : undefined} />}>
      <RowLabel shimmer={running && activity.active}>{retired ? t('host.working') : label}</RowLabel>
      {elapsed && !retired && <span className="min-w-0 truncate text-fg-3" title={elapsed}>{elapsed}</span>}
      {blocks.length > 0 && <ChevronRight className={cn('size-3 shrink-0 self-center transition-transform', open && 'rotate-90')} strokeWidth={1.75} />}
    </Collapsible.Trigger>
  );
  return (
    <Collapsible.Root open={open} onOpenChange={toggle} className={cn(
      'group flex min-w-0 flex-col transition-[margin-bottom] duration-(--dur-open) ease-out',
      // The root is the reply stack's flex item; an inner margin cannot cancel its sibling gap.
      retired && blocks.length === 0 && '-mb-gap [transition-delay:var(--dur-open)]',
    )} data-open={open || undefined}>
      {mounted.current && (
        // The clip reserves the head's hit area like the panel below; a flex column stretches the button to the full row
        <div inert={retired} className={cn('-mx-hit grid transition-[grid-template-rows,opacity] duration-(--dur-open) ease-out',
          retired ? 'grid-rows-[0fr] opacity-0 [transition-delay:var(--dur-open),0s]' : 'grid-rows-[1fr] opacity-100')}>
          <div className="flex min-h-0 flex-col overflow-hidden px-hit"><EntranceOnce id="fold-head">{head}</EntranceOnce></div>
        </div>
      )}
      {blocks.length > 0 && (
        <AutoFoldContext.Provider value={store}>
          {parts.map((part, i) => {
            // A steered prompt keeps the process prose gap open or closed: between two parts while they show, under
            // the head (or the previous card) once they have folded away
            if (part.type === 'steer') return (
              <div key={part.id} className="flex min-w-0 flex-col pt-(--process-prose-gap)">
                <EntranceOnce id={itemEntrance(part.id)}><SteeredMessage block={part.block} /></EntranceOnce>
              </div>
            );
            // The padding folds with its part: the head's offset on the first, the prose gap after a card, the tail on the last
            const body = (
              <div className={cn(!retired && (i === 0 ? 'pt-1' : 'pt-(--process-prose-gap)'), !retired && i === parts.length - 1 && 'pb-1.5')}>
                <ProcessHistory><ProcessBlocks blocks={blocks} items={part.items} /></ProcessHistory>
              </div>
            );
            // Nested rows extend their hit area beyond the text column; the clip reserves the turn padding (wider than
            // the hit outset) so they keep their edges. The first part is the root's panel, the rest follow it
            return i === parts.findIndex(p => p.type === 'items')
              ? <Collapsible.Panel key={part.id} className="-mx-pad [&>div]:px-pad">{body}</Collapsible.Panel>
              : <Collapsible.Section key={part.id} open={open} className="-mx-pad [&>div]:px-pad">{body}</Collapsible.Section>;
          })}
        </AutoFoldContext.Provider>
      )}
    </Collapsible.Root>
  );
}

type FoldPart = { type: 'items'; id: string; items: ProcessItem[] } | { type: 'steer'; id: string; block: SteerBlock };

// The turn's process cut at each steered prompt: runs of items fold, the prompts between them stay. A part is keyed by
// its first item, so a prompt steered into a live turn leaves the earlier part mounted and starts a new one after it
function splitAtSteers(items: ProcessItem[]): FoldPart[] {
  const out: FoldPart[] = [];
  for (const item of items) {
    const last = out[out.length - 1];
    if (item.type === 'block' && item.block.type === 'steer') out.push({ type: 'steer', id: item.id, block: item.block });
    else if (last?.type === 'items') last.items.push(item);
    else out.push({ type: 'items', id: item.id, items: [item] });
  }
  return out;
}

// The outer fold's close (--dur-close) and a frame or two: rows inside hold still until it has finished
const SETTLE_MS = 400;

// The folds of a turn's process items, by `foldStates`. When the turn's fold closes on its own the rows hold still
// until that close has run (`settle`); a manual toggle is remembered per row
function useProcessFolds(items: ProcessItem[], settle: boolean, live: boolean, following: boolean, memoryKey: string | undefined): AutoFoldStore {
  const { autoExpand } = useAppearance();
  const [rows, setRows] = useState<Record<string, boolean>>({});
  const [store] = useState(() => new AutoFoldStore(() => {}));
  store.toggle = (id, next) => {
    setRows(current => ({ ...current, [id]: next }));
    if (memoryKey) rememberFold(`${memoryKey}:${id}`, next);
  };
  const [, wake] = useState(0);
  const doneAt = useRef<Map<string, number> | undefined>(undefined);
  const held = useRef(new Set<string>());
  const settleUntil = useRef(0);
  const wasLive = useRef(live);
  const now = performance.now();
  if (wasLive.current && !live && settle) settleUntil.current = now + SETTLE_MS;
  wasLive.current = live;
  const running = live || now < settleUntil.current;
  // Items already finished when the turn mounted count as long done, so opening a live session does not flash them
  const first = doneAt.current === undefined;
  const { folds, open, wake: due } = foldStates({
    items, now, live: running, autoExpand: autoExpand === 'on', following, doneAt: doneAt.current ??= new Map(), first, held: held.current,
    manual: id => rows[id] ?? (memoryKey ? rememberedFold(`${memoryKey}:${id}`) : undefined),
  });
  const next = running && !live ? Math.min(due, settleUntil.current) : due;
  held.current = open;
  // Written before the rows render; rows that skip rendering (memoized on their block) hear it after commit
  store.write(folds);
  useLayoutEffect(() => store.notify());
  useEffect(() => {
    if (next === Infinity) return;
    const timer = setTimeout(() => wake(n => n + 1), Math.max(0, next - performance.now()));
    return () => clearTimeout(timer);
  }, [next]);
  return store;
}

// Process details retain static icons; only the currently running verb shimmers. Each item enters once under its
// own id at the list level, whatever row component draws it: a single call that becomes a group keeps its first
// call's id, and a remounted transcript replays nothing. Unkeyed blocks take their transcript position
function ProcessBlocks({ blocks, items }: { blocks: AgentBlock[]; items?: ProcessItem[] }) {
  return (items ?? groupProcess(blocks)).map(item => (
    <EntranceOnce key={item.id} id={itemEntrance(item.id)}>
      <AutoFoldItem id={item.id}>{item.type === 'group' ? <ToolGroup kind={item.kind} blocks={item.blocks} />
        : item.block.type === 'tool_call' ? <ToolCall block={item.block} grouped />
        : item.block.type === 'text' ? <Prose block={item.block} />
        : <LineBlock block={item.block} />}</AutoFoldItem>
    </EntranceOnce>
  ));
}

function LineBlock({ block }: { block: AgentBlock }) {
  if (block.type === 'notice') return <NoticeRow block={block} />;
  if (block.type === 'thought') return <Thought block={block} />;
  if (block.type === 'plan') return <Plan block={block} />;
  if (block.type === 'tool_call') return <ToolCall block={block} />;
  if (block.type === 'compaction') return <Compaction block={block} />;
  if (block.type === 'steer') return <SteeredMessage block={block} />;
  if (block.type === 'image') return <AgentImage image={block} />;
  // The open card is pinned above the composer by the shell; only a resolved one has a place in the message
  if (block.type === 'question') return block.outcome ? <QuestionRecord block={block} /> : null;
  return null;
}

// Context compaction is an action row like a tool call: same lead slot (the context panel's compact icon), and the
// live status shimmers in place until the agent reports the outcome, while the icon's four arrows keep pushing inward. The host turns structured compaction_update and
// adapter prose (rust `compaction_text`) into the same block, so every agent shares this one presentation.
export function Compaction({ block }: { block: CompactionBlock }) {
  const { toolLine } = useAppearance();
  const row = useRef<HTMLElement>(null);
  const phase = useCompactionPhase(block, row);
  // A completion still waiting for the arrows' rest beat keeps the running look, so label and icon change together
  const running = block.status === 'in_progress' || phase === 'finishing';
  const label = running ? t('turns.compacting') : block.status === 'completed' ? t('turns.compacted') : block.status === 'failed' ? t('turns.compactFailed') : t('turns.compactCancelled');
  const Icon = block.status === 'failed' ? TriangleAlert : Shrink;
  return (
    <Row ref={row} tone="action" className={cn(phase === 'settling' && 'compaction-settle')} lead={toolLine === 'text' ? undefined : <Icon className={cn('size-icon', running && 'compacting-icon')} strokeWidth={1.5} />}>
      <RowLabel shimmer={running}>{label}</RowLabel>
      {block.status === 'failed' && block.error && <RowTarget className="text-fg-3">{block.error}</RowTarget>}
    </Row>
  );
}

// Compaction ids seen running in this webview. Kept at module level because the row may remount between the running
// and the completed render (the turn's process grouping changes when it ends), and a component ref would lose it.
const compactingSeen = new Set<string>();
// `acp-compact-in` timing in motion.css: one 1.4 s cycle, the last arrow (0.36 s delay) back at rest from 1.2 s in,
// so every arrow sits at rest from 1.2 s to the cycle's end. The settle starts there and from that rest pose.
const COMPACTING_CYCLE_MS = 1400;
const COMPACTING_REST_MS = 1200;
// `compaction-settle` length, after which the class is dropped again
const COMPACTION_SETTLE_MS = 900;

type CompactionPhase = 'idle' | 'finishing' | 'settling';

// Drives the completion beat of a compaction watched while running: `finishing` lets the running loop play on until
// all four arrows are at rest, then `settling` clamps them shut once. Restored history (never seen running) stays
// `idle`, and the id leaves the set on its first settle, so a later remount does not replay it. The class is removed
// afterwards: a hidden webview restarts every CSS animation when it is shown again.
function useCompactionPhase(block: CompactionBlock, row: RefObject<HTMLElement | null>): CompactionPhase {
  const [phase, setPhase] = useState<CompactionPhase>('idle');
  useLayoutEffect(() => {
    if (block.status === 'in_progress') { compactingSeen.add(block.id); return; }
    if (!compactingSeen.delete(block.id) || block.status !== 'completed') return;
    // The running loop on this element, if it survived to here (absent after a remount or with motion off)
    const loop = row.current?.querySelector('.compacting-icon path')?.getAnimations()[0];
    const at = typeof loop?.currentTime === 'number' ? loop.currentTime % COMPACTING_CYCLE_MS : undefined;
    const wait = at === undefined || at >= COMPACTING_REST_MS ? 0 : COMPACTING_REST_MS - at;
    setPhase(wait ? 'finishing' : 'settling');
    const timers = [
      setTimeout(() => setPhase('settling'), wait),
      setTimeout(() => setPhase('idle'), wait + COMPACTION_SETTLE_MS),
    ];
    return () => timers.forEach(clearTimeout);
  }, [block.id, block.status, row]);
  return phase;
}

function Block({ block, onPermission }: { block: AgentBlock; onPermission: OnPermission }) {
  if (block.type === 'text') return <Prose block={block} />;
  if (block.type === 'steer') return <SteeredMessage block={block} />;
  if (block.type === 'permission') return block.planId ? null : <Permission block={block} onChoose={id => onPermission(block.id, id)} />;
  return <LineBlock block={block} />;
}
