import { WebMetrics, Metrics } from '../design/Tokens';
import { AgentBlock, Turn } from '../wire/Wire';
import { asTurn, Attachment, Block, SubagentSummary, TurnX } from '../wire/Blocks';
import {
  clockStart, CodexSplit, elapsedLabel, FoldPart, groupProcess, LiveActivity, liveActivity, observed, PlanSection,
  processParts, splitCodexBlocks, splitPlanSections, turnOutcome
} from '../logic/Folding';
import { delegatedIds, Placement, placeNodes } from '../logic/Subagents';
import { t } from '../store/L10n';
import { ThreadContext } from '../store/Thread';
import { Bubble } from './Bubble';
import { PermissionCard, PlanDocumentCard, showsTurnActions, TurnActions } from './Cards';
import { Prose } from './Prose';
import { GoalMilestone, itemSig, NoticeRow, OutcomeRow, ProcessHistory, SteeredMessage } from './ProcessViews';
import { folds, RowSpec, TapRow } from './Rows';
import { ChildPermissions, DelegatedSubagents, SubagentGroup } from './SubagentViews';
import { blockSig, ImageResults } from './ToolViews';

// The transcript's turns (apps/ios Transcript/AgentMessage.swift, chat/Turns.tsx under fold = codex; apps/android
// ui/Transcript.kt): the prompt as a right-hand bubble, each agent turn as one process fold whose head carries the live
// activity or the finished time, the reply outside it, permission cards after the reply, the outcome row last. A saved
// plan cuts the turn: its card stands between the stretch before it and the continuation after it.

/** The sent prompt (Cards.swift UserMessageCard): the bubble with its attachments and command / persona pills */
@ComponentV2
export struct UserBubble {
  @Param text: string = '';
  /** The whole turn, when the chat has it: attachments, the recorded command, `auto` */
  @Param turn: Turn | undefined = undefined;
  /** The agent's advertised command names (observed: a kept row keeps its params), and the personas it can summon */
  @Param thread: ThreadContext | undefined = undefined;
  @Param summons: string[] = [];

  attachments(): Attachment[] {
    return this.turn !== undefined ? (asTurn(this.turn).attachments ?? []) : [];
  }

  /** A recorded command keeps its pill even after the agent stops advertising it */
  names(): string[] {
    const recorded = this.turn !== undefined ? asTurn(this.turn).command : undefined;
    const commands = this.thread?.commands ?? [];
    return typeof recorded === 'string' && recorded.length > 0 ? commands.concat([recorded]) : commands;
  }

  build() {
    Bubble({
      text: this.turn?.text ?? this.text, attachments: this.attachments(),
      auto: this.turn !== undefined && asTurn(this.turn).auto === true, commands: this.names(), summons: this.summons
    })
  }
}

/**
 * A stretch as its own message: only the continuation owns the live activity and the turn's outcome, and with more than
 * one stretch none has timing of its own (repeating the whole turn's duration would lie)
 */
function sectionTurn(turn: TurnX, section: PlanSection, last: boolean, split: boolean): TurnX {
  const out: TurnX = { role: turn.role };
  out.id = turn.id;
  out.blocks = section.blocks;
  out.observation = turn.observation;
  out.retry = turn.retry;
  out.usage = turn.usage;
  if (last) {
    out.stop = turn.stop;
    out.error = turn.error;
    out.command = turn.command;
  }
  if (!split) {
    out.startedAt = turn.startedAt;
    out.respondedAt = turn.respondedAt;
    out.endedAt = turn.endedAt;
  }
  return out;
}

/** A section's identity for a ForEach key: its key and what its message would show differently */
function sectionSig(s: PlanSection): string {
  return `${s.key}|${s.blocks.map(blockSig).join(',')}|${s.plan !== undefined ? blockSig(s.plan) : ''}`;
}

/** One agent turn: its stretches (each a CodexMessage) with the plan cards between them, then the reply's actions */
@ComponentV2
export struct AgentMessage {
  @Param turn: Turn = { role: 'agent' };
  /** The turn's place in the session (fork from here) and the scope its fold choices are kept under */
  @Param index: number = 0;
  @Param scope: string = '';
  /**
   * What the turn reads from the view (subagents, controls, agent, which turn runs, which one Retry answers): observed,
   * since a row whose turn did not change keeps the params it was built with
   */
  @Param thread: ThreadContext = new ThreadContext();
  @Param now: () => number = () => Date.now();
  /** Fork and retry are offered when the chat wires them */
  @Param actions: boolean = false;
  @Event onPermission: (blockId: string, optionId: string) => void = (blockId: string, optionId: string) => {};
  @Event onBuild: (planId: string, optionId: string, configId: string, value: string) => void =
    (planId: string, optionId: string, configId: string, value: string) => {};
  @Event onFork: (turnIndex: number) => void = (turnIndex: number) => {};
  @Event onRetry: () => void = () => {};

  turnX(): TurnX {
    return asTurn(this.turn);
  }

  /** This turn is the one running */
  get running(): boolean {
    return this.thread.live === this.index;
  }

  /** The view's subagents: every node, for counts and provenance; this turn's are those announced in it */
  get subagents(): SubagentSummary[] {
    return this.thread.subagents;
  }

  blocks(): Block[] {
    return (this.turn.blocks ?? []) as Block[];
  }

  turnNodes(): SubagentSummary[] {
    return this.subagents.filter((n: SubagentSummary) => n.turnIndex === this.index);
  }

  @Computed
  get placement(): Placement {
    return placeNodes(this.turnNodes(), delegatedIds(this.blocks(), this.turnNodes()));
  }

  @Computed
  get sections(): PlanSection[] {
    return splitPlanSections(this.blocks(), delegatedIds(this.blocks(), this.turnNodes()));
  }

  scopeKey(): string {
    return `${this.scope}:${this.index}`;
  }

  /**
   * Whether a stretch draws a message: a plan approval renders on its card, so a stretch holding nothing else while the
   * card waits has no working row to show; the last stretch still draws a live turn, an outcome or unplaced children
   */
  shows(section: PlanSection, i: number): boolean {
    const visible = section.blocks.some((b: Block) => b.type !== 'permission' || b.planId === undefined);
    const planWait = section.blocks.some((b: Block) => b.type === 'permission' && b.planId !== undefined);
    const last = i === this.sections.length - 1;
    return visible || (last && ((this.running && !planWait) || turnOutcome(this.turnX()) !== undefined ||
      this.placement.rest.length > 0));
  }

  approval(plan: Block): Block | undefined {
    return this.blocks().find((b: Block) => b.type === 'permission' && b.planId === plan.id);
  }

  build() {
    Column({ space: Metrics.gap }) {
      ForEach(this.sections, (section: PlanSection, i: number) => {
        Column({ space: Metrics.gap }) {
          if (this.shows(section, i)) {
            CodexMessage({
              turn: sectionTurn(this.turnX(), section, i === this.sections.length - 1, this.sections.length > 1),
              scopeKey: this.sections.length > 1 && i > 0 ? `${this.scopeKey()}/${section.key}` : this.scopeKey(),
              running: i === this.sections.length - 1 && this.running,
              now: this.now,
              rest: i === this.sections.length - 1 ? this.placement.rest : [],
              delegations: this.placement.byId,
              all: this.subagents,
              onPermission: this.onPermission
            })
          }
          if (section.plan !== undefined) {
            PlanDocumentCard({
              block: section.plan, permission: this.approval(section.plan), running: this.running,
              controls: this.thread.controls, agent: this.thread.agent,
              onBuild: (optionId: string, configId: string, value: string) =>
                this.onBuild(section.plan?.id ?? '', optionId, configId, value),
              onPermission: this.onPermission
            })
          }
        }
        .width('100%')
      }, (section: PlanSection) => sectionSig(section))
      if (showsTurnActions(this.turnX(), this.running)) {
        TurnActions({
          turn: this.turnX(), retry: this.thread.retry === this.index, actions: this.actions, title: this.thread.title,
          agent: this.thread.agent, controls: this.thread.controls, onRetry: () => this.onRetry(),
          onFork: () => this.onFork(this.index)
        })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

/**
 * One stretch of a turn as its own message (CodexMessage): goal milestones, the process fold, notices, the reply,
 * permission cards (a plan's approval renders on the plan card) and the outcome row
 */
@ComponentV2
struct CodexMessage {
  @Param turn: TurnX = { role: 'agent' };
  @Param scopeKey: string = '';
  @Param running: boolean = false;
  @Param now: () => number = () => Date.now();
  /** The turn's children with no delegation row to sit at: they follow the process, before the reply */
  @Param rest: SubagentSummary[] = [];
  @Param delegations: Map<string, SubagentSummary[]> = new Map<string, SubagentSummary[]>();
  @Param all: SubagentSummary[] = [];
  @Event onPermission: (blockId: string, optionId: string) => void = (blockId: string, optionId: string) => {};

  @Computed
  get split(): CodexSplit {
    // A call whose observation was lost reads as settled before anything groups or draws it
    return splitCodexBlocks((this.turn.blocks ?? []).map(observed));
  }

  hasTools(): boolean {
    return (this.turn.blocks ?? []).some((b: AgentBlock) => b.type === 'tool_call');
  }

  build() {
    Column({ space: Metrics.gap }) {
      // Goal milestones before anything else open the turn above the fold
      ForEach(this.split.goalsLead, (b: Block) => {
        GoalMilestone({ block: b })
      }, (b: Block) => blockSig(b))
      CodexFold({
        turn: this.turn, blocks: this.split.process, running: this.running, hasTools: this.hasTools(), scopeKey: this.scopeKey,
        now: this.now, delegations: this.delegations, all: this.all, onPermission: this.onPermission
      })
      ForEach(this.split.notices, (b: Block) => {
        NoticeRow({ block: b, scope: this.scopeKey, now: this.now })
      }, (b: Block) => blockSig(b))
      if (this.rest.length > 0) {
        SubagentGroup({ nodes: this.rest, all: this.all, now: this.now })
        ChildPermissions({ nodes: this.rest, all: this.all, onPermission: this.onPermission })
      }
      ForEach(this.split.reply, (b: Block) => {
        if ((b.markdown ?? '').trim().length > 0) {
          Prose({ markdown: b.markdown ?? '', streaming: b.streaming === true })
        }
      }, (b: Block, i: number) => `${i}:${blockSig(b)}`)
      // Later milestones (paused, met, cleared …) close the reply they followed
      ForEach(this.split.goals, (b: Block) => {
        GoalMilestone({ block: b })
      }, (b: Block) => blockSig(b))
      ForEach(this.split.permissions.filter((b: Block) => b.planId === undefined), (b: Block) => {
        PermissionCard({ block: b, onPick: (optionId: string) => this.onPermission(b.id ?? '', optionId) })
      }, (b: Block) => blockSig(b))
      if (!this.running && turnOutcome(this.turn) !== undefined) {
        OutcomeRow({ turn: this.turn, outcome: turnOutcome(this.turn) ?? '' })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

/** Below this a finished turn shows the bare Done label */
const HALF_SECOND_MS: number = 500;

/** A part's identity for a ForEach key */
function partSig(p: FoldPart): string {
  return `${p.kind}:${p.id}|${p.items.map(itemSig).join(',')}|${p.ids.join(',')}|${p.block !== undefined ? blockSig(p.block) : ''}`;
}

/**
 * The process fold: open while the turn works, closed once it is done; a tap on the head always wins (Turns.tsx
 * CodexFold). A turn with no process worth a head (a plain reply, or only a compaction status) shows no head. A steered
 * prompt and a delegation's rows stay in view while the parts around them fold
 */
@ComponentV2
struct CodexFold {
  @Param turn: TurnX = { role: 'agent' };
  @Param blocks: Block[] = [];
  @Param running: boolean = false;
  @Param hasTools: boolean = false;
  @Param scopeKey: string = '';
  @Param now: () => number = () => Date.now();
  @Param delegations: Map<string, SubagentSummary[]> = new Map<string, SubagentSummary[]>();
  @Param all: SubagentSummary[] = [];
  @Event onPermission: (blockId: string, optionId: string) => void = (blockId: string, optionId: string) => {};

  retired(): boolean {
    return !this.running && !this.hasTools && this.blocks.every((b: Block) => b.type === 'compaction');
  }

  open(): boolean {
    return this.retired() || folds.isOpen(`${this.scopeKey}:fold`, this.running);
  }

  @Computed
  get parts(): FoldPart[] {
    return processParts(groupProcess(this.blocks));
  }

  foldable(): boolean {
    return this.parts.some((p: FoldPart) => p.kind === 'items');
  }

  /** The head: the Orb (or hand, plan, question) and the activity while live; a tick, ✗ or warning and the time once done */
  head(): RowSpec {
    const activity: LiveActivity = liveActivity(this.turn);
    const outcome = turnOutcome(this.turn);
    const s = new RowSpec();
    s.height = WebMetrics.chatRow;
    s.iconTint = $r('app.color.fg2');
    if (!this.running) {
      s.icon = this.turn.stop === 'cancelled' ? 'x' : outcome !== undefined ? 'triangleAlert' : 'check';
    } else if (activity.lead === 'orb') {
      s.lead = 'orb';
    } else {
      s.icon = activity.lead === 'hand' ? 'hand' : activity.lead === 'question' ? 'messageCircleQuestion' : 'listTodo';
    }
    s.label = this.running ? activity.label : (outcome ?? t('turns.done'));
    s.shimmer = this.running && activity.active;
    const since = clockStart(this.turn);
    if (this.running && since !== undefined) {
      s.since = since;
      s.now = this.now;
    }
    // A turn that rounds to 0s shows the bare Done label, as the live counter stays hidden for its first second
    const end = this.turn.endedAt;
    if (!this.running && this.turn.startedAt !== undefined && end !== undefined && (since === undefined || end - since >= HALF_SECOND_MS)) {
      s.elapsed = elapsedLabel(this.turn);
    }
    if (this.foldable()) {
      s.chevron = 'right';
      s.open = this.open();
    }
    return s;
  }

  partTop(i: number): number {
    return this.retired() ? 0 : i === 0 ? Metrics.s1 : WebMetrics.processProseGap;
  }

  partBottom(i: number): number {
    return !this.retired() && i === this.parts.length - 1 ? WebMetrics.s1_5 : 0;
  }

  build() {
    if (!(this.retired() && this.blocks.length === 0)) {
      Column() {
        if (!this.retired()) {
          TapRow({
            spec: this.head(), onTap: () => {
              if (this.foldable()) {
                folds.toggle(this.getUIContext(), `${this.scopeKey}:fold`, this.running);
              }
            }
          })
        }
        ForEach(this.parts, (part: FoldPart, i: number) => {
          if (part.kind === 'steer') {
            Column() {
              SteeredMessage({ block: part.block ?? { type: 'steer' } })
            }
            .padding({ top: WebMetrics.processProseGap })
            .width('100%')
          } else if (part.kind === 'subagents') {
            Column() {
              DelegatedSubagents({
                ids: part.ids, delegations: this.delegations, all: this.all, now: this.now, onPermission: this.onPermission
              })
            }
            .padding({ top: WebMetrics.processProseGap })
            .width('100%')
          } else if (part.kind === 'media') {
            Column() {
              ImageResults({ block: part.block ?? { type: 'tool_call' } })
            }
            .padding({ top: Metrics.gap, bottom: this.partBottom(i) })
            .width('100%')
          } else if (this.open()) {
            Column() {
              ProcessHistory({ items: part.items, scope: this.scopeKey, now: this.now })
            }
            .padding({ top: this.partTop(i), bottom: this.partBottom(i) })
            .width('100%')
            .transition(TransitionEffect.OPACITY.animation({ duration: WebMetrics.foldMs, curve: Curve.Smooth }))
          }
        }, (part: FoldPart) => partSig(part))
      }
      .alignItems(HorizontalAlign.Start)
      .width('100%')
    }
  }
}
