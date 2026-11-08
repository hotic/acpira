import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, Question, QuestionOption, TurnX } from '../wire/Blocks';
import {
  firstLine, goalEventLabel, goalEventStatus, goalRowDetail, ProcessItem, tp
} from '../logic/Folding';
import { t } from '../store/L10n';
import { Bubble } from './Bubble';
import { Prose } from './Prose';
import { AcpRow, Disclosure, RowSpec } from './Rows';
import { AgentImage, blockSig, PlanDetails, RetryRow, ToolCallView, ToolGroupView } from './ToolViews';

// chat/Thought.tsx, Plan.tsx, ProcessHistory.tsx and the Compaction / NoticeRow / Outcome rows of Turns.tsx, after the
// iOS app's Transcript/ProcessViews.swift and GoalViews.swift (apps/android ui/ProcessViews.kt).

function isProse(item: ProcessItem): boolean {
  return item.block !== undefined && (item.block.type === 'text' || item.block.type === 'steer');
}

/** An item's identity for a ForEach key: its id and what its rows would show differently */
export function itemSig(item: ProcessItem): string {
  if (item.block !== undefined) {
    return `${item.id}|${blockSig(item.block)}`;
  }
  return `${item.id}|${item.group}|${item.blocks.map(blockSig).join(',')}`;
}

/**
 * `process-rows`: rows and dense rows shrink to --process-row, items sit --process-row-gap apart, and prose inside the
 * process keeps --process-prose-gap from its neighbours
 */
@ComponentV2
export struct ProcessHistory {
  @Param items: ProcessItem[] = [];
  @Param scope: string = '';
  @Param now: () => number = () => Date.now();

  gapBefore(i: number): number {
    const prose = isProse(this.items[i]);
    const afterProse = i > 0 && isProse(this.items[i - 1]);
    return (prose && i > 0) || afterProse ? WebMetrics.processProseGap - WebMetrics.processRowGap : 0;
  }

  build() {
    Column({ space: WebMetrics.processRowGap }) {
      ForEach(this.items, (item: ProcessItem, i: number) => {
        Column() {
          ProcessItemView({ item: item, scope: this.scope, now: this.now })
        }
        .padding({ top: this.gapBefore(i) })
        .width('100%')
      }, (item: ProcessItem) => itemSig(item))
    }
    .width('100%')
  }
}

/** One process item: a read / edit group, a single call, prose, or a line block */
@ComponentV2
export struct ProcessItemView {
  @Param item: ProcessItem = new ProcessItem();
  @Param scope: string = '';
  @Param now: () => number = () => Date.now();

  build() {
    if (this.item.group.length > 0) {
      ToolGroupView({ kind: this.item.group, blocks: this.item.blocks, scope: this.scope, rowH: WebMetrics.processRow })
    } else if (this.item.block !== undefined) {
      if (this.item.block.type === 'tool_call') {
        ToolCallView({ block: this.item.block, grouped: true, scope: this.scope, rowH: WebMetrics.processRow, now: this.now })
      } else if (this.item.block.type === 'text') {
        if ((this.item.block.markdown ?? '').trim().length > 0) {
          Prose({ markdown: (this.item.block.markdown ?? '').trimEnd(), streaming: this.item.block.streaming === true })
        }
      } else {
        LineBlockView({ block: this.item.block, scope: this.scope, rowH: WebMetrics.processRow, now: this.now })
      }
    }
  }
}

@ComponentV2
export struct LineBlockView {
  @Param block: Block = { type: 'unknown' };
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;
  @Param now: () => number = () => Date.now();

  planHead(): RowSpec {
    const s = new RowSpec();
    s.label = t('plan.title');
    s.height = this.rowH;
    return s;
  }

  build() {
    if (this.block.type === 'notice') {
      NoticeRow({ block: this.block, scope: this.scope, rowH: this.rowH, now: this.now })
    } else if (this.block.type === 'goal') {
      GoalMilestone({ block: this.block, rowH: this.rowH })
    } else if (this.block.type === 'thought') {
      ThoughtView({ block: this.block, scope: this.scope, rowH: this.rowH })
    } else if (this.block.type === 'plan') {
      PlanDetails({
        entries: this.block.entries ?? [], head: this.planHead(),
        foldKey: `${this.scope}:plan::${(this.block.entries ?? []).length}:${(this.block.entries ?? [])[0]?.title ?? ''}`
      })
    } else if (this.block.type === 'tool_call') {
      ToolCallView({ block: this.block, scope: this.scope, rowH: this.rowH, now: this.now })
    } else if (this.block.type === 'compaction') {
      CompactionRow({ block: this.block, rowH: this.rowH })
    } else if (this.block.type === 'steer') {
      SteeredMessage({ block: this.block })
    } else if (this.block.type === 'image') {
      AgentImage({ uri: this.block.uri ?? '', name: '' })
    } else if (this.block.type === 'question') {
      // The open card waits for an answer in the dock; only a resolved one has a place in the message
      if (this.block.outcome !== undefined) {
        QuestionRecord({ block: this.block, scope: this.scope, rowH: this.rowH })
      }
    } else if (this.block.type !== 'text' && this.block.type !== 'permission' && this.block.type !== 'plan_document') {
      UnknownBlockRow({ type: this.block.type, rowH: this.rowH })
    }
  }
}

/** A prompt sent while the turn ran: the user's bubble, at the thread's own width */
@ComponentV2
export struct SteeredMessage {
  @Param block: Block = { type: 'steer' };

  build() {
    Bubble({ text: this.block.text ?? '', attachments: this.block.attachments ?? [] })
  }
}

// Thought

/**
 * A thought: one action row (Thinking while it streams), its text in thought-tone prose inside a bounded scrollport that
 * follows the stream, an edge faded while more text lies beyond it
 */
@ComponentV2
export struct ThoughtView {
  @Param block: Block = { type: 'thought' };
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;
  @Local natural: number = 0;
  private scroller: Scroller = new Scroller();

  spec(): RowSpec {
    const live = this.block.streaming === true;
    const s = new RowSpec();
    s.icon = 'brain';
    s.action = true;
    s.height = this.rowH;
    s.label = live ? t('host.thinking') : t('thought.label');
    s.shimmer = live;
    return s;
  }

  build() {
    // Thoughts carry no id; the start stamp names them within the turn
    Disclosure({ spec: this.spec(), foldKey: `${this.scope}:thought:${Math.floor(this.block.startedAt ?? 0)}` }) {
      Stack({ alignContent: Alignment.Bottom }) {
        Scroll(this.scroller) {
          Column() {
            Prose({ markdown: (this.block.text ?? '').trimEnd(), tone: 'thought', streaming: this.block.streaming === true })
          }
          .onAreaChange((before: Area, after: Area) => {
            this.natural = Number(after.height);
            if (this.block.streaming === true) {
              this.scroller.scrollEdge(Edge.Bottom);
            }
          })
        }
        .scrollBar(BarState.Off)
        .edgeEffect(EdgeEffect.None)
        .align(Alignment.TopStart)
        .constraintSize({ maxHeight: this.rowH * WebMetrics.thoughtRows })
        .width('100%')
        if (this.natural > this.rowH * WebMetrics.thoughtRows) {
          Column()
            .height(WebMetrics.scrollFade)
            .width('100%')
            .linearGradient({ angle: 180, colors: [[Color.Transparent, 0], [$r('app.color.bg0'), 1]] })
            .hitTestBehavior(HitTestMode.None)
        }
      }
      .width('100%')
    }
  }
}

// Compaction

/** Context compaction as an action row (Turns.tsx Compaction): the live status shimmers, a failure names its error */
@ComponentV2
export struct CompactionRow {
  @Param block: Block = { type: 'compaction' };
  @Param rowH: number = WebMetrics.chatRow;

  spec(): RowSpec {
    const status = this.block.status;
    const s = new RowSpec();
    s.icon = status === 'failed' ? 'triangleAlert' : 'shrink';
    s.action = true;
    s.height = this.rowH;
    s.label = t(status === 'in_progress' ? 'turns.compacting' : status === 'completed' ? 'turns.compacted' :
      status === 'failed' ? 'turns.compactFailed' : 'turns.compactCancelled');
    s.shimmer = status === 'in_progress';
    if (status === 'failed' && (this.block.error ?? '').length > 0) {
      s.target = this.block.error ?? '';
      s.targetInk = $r('app.color.fg3');
    }
    return s;
  }

  build() {
    AcpRow({ spec: this.spec() })
  }
}

// Notices, outcome, goals

/** Workspace hook notices (`acp/session/hooks.rs`) carry this id prefix and the gate's shield glyph */
const HOOK_NOTICE: string = 'hooks:';

function noticeGlyph(b: Block, error: boolean): string {
  if ((b.id ?? '').startsWith(HOOK_NOTICE)) {
    return error ? 'shieldX' : 'shieldAlert';
  }
  return b.info === true ? 'info' : 'triangleAlert';
}

/** A details line that names a file (`- path`): its path, else '' */
function noticePath(line: string): string {
  const m = line.match(/^\s*-\s+(\S+)$/);
  return m !== null && (m[1].includes('/') || m[1].includes('.')) ? m[1] : '';
}

/**
 * One row per failure (Turns.tsx NoticeRow). A notice with details folds like a thought, the details on the rail once
 * open; an error opens by default and marks only its glyph with the warn tone. An info notice only informs
 */
@ComponentV2
export struct NoticeRow {
  @Param block: Block = { type: 'notice' };
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;
  @Param now: () => number = () => Date.now();

  error(): boolean {
    return this.block.severity === 'error';
  }

  spec(): RowSpec {
    const s = new RowSpec();
    s.icon = noticeGlyph(this.block, this.error());
    s.iconTint = this.error() ? $r('app.color.warn') : $r('app.color.fg3');
    s.height = this.rowH;
    s.label = this.block.title ?? '';
    s.wrap = true;
    s.labelInk = this.error() ? $r('app.color.fg2') : $r('app.color.fg3');
    return s;
  }

  build() {
    if (this.block.retry !== undefined) {
      RetryRow({ noticeId: this.block.id ?? '', log: this.block.retry, scope: this.scope, rowH: this.rowH, now: this.now })
    } else if ((this.block.details ?? '').trim().length > 0) {
      Disclosure({ spec: this.spec(), foldKey: `${this.scope}:notice:${this.block.id ?? ''}`, defaultOpen: this.error() }) {
        Column() {
          ForEach((this.block.details ?? '').trimEnd().split('\n'), (line: string) => {
            if (noticePath(line).length > 0) {
              Row({ space: Metrics.gap }) {
                AcpRow({ spec: this.fileRow(noticePath(line)) })
              }
            } else {
              Text(line.length > 0 ? line : ' ')
                .fontSize(WebType.text3)
                .lineHeight(WebType.text3Line)
                .fontColor($r('app.color.fg3'))
                .width('100%')
            }
          }, (line: string, i: number) => `${i}:${line}`)
        }
        .alignItems(HorizontalAlign.Start)
        .width('100%')
      }
    } else {
      AcpRow({ spec: this.spec() })
    }
  }

  fileRow(path: string): RowSpec {
    const s = new RowSpec();
    s.icon = 'fileText';
    s.height = WebMetrics.rowDense;
    s.target = path;
    s.targetMono = true;
    s.targetInk = $r('app.color.fg2');
    return s;
  }
}

/** One faint row closing the message: how the turn ended, the error's own words as the target */
@ComponentV2
export struct OutcomeRow {
  @Param turn: TurnX = { role: 'agent' };
  @Param outcome: string = '';

  spec(): RowSpec {
    // A warning glyph for the short stops, none for "stopped" / "no reply"
    const warn = this.turn.stop !== 'cancelled' && this.turn.stop !== 'end_turn';
    const s = new RowSpec();
    s.icon = warn ? 'triangleAlert' : '';
    s.lead = warn ? 'icon' : 'none';
    s.height = WebMetrics.chatRow;
    s.label = this.outcome;
    if (this.turn.stop === 'error' && (this.turn.error?.message ?? '').trim().length > 0) {
      s.target = this.turn.error?.message ?? '';
      s.targetInk = $r('app.color.fg3');
    }
    return s;
  }

  build() {
    AcpRow({ spec: this.spec() })
  }
}

/** A milestone (GoalViews.swift GoalRow): the event, the objective's first line as the target, the spend of a stretch */
@ComponentV2
export struct GoalMilestone {
  @Param block: Block = { type: 'goal' };
  @Param rowH: number = WebMetrics.chatRow;

  spec(): RowSpec {
    const status = goalEventStatus(this.block.event);
    const s = new RowSpec();
    s.icon = status === 'complete' ? 'circleCheck' : status === 'blocked' || status === 'limited' ? 'triangleAlert' : 'goal';
    s.iconTint = status === 'complete' ? $r('app.color.ok') :
      status === 'blocked' || status === 'limited' ? $r('app.color.warn') : $r('app.color.fg3');
    s.height = this.rowH;
    s.label = goalEventLabel(this.block.event);
    s.target = firstLine(this.block.objective);
    s.meta = goalRowDetail(this.block) ?? '';
    return s;
  }

  build() {
    AcpRow({ spec: this.spec() })
  }
}

/** An answer's option labels (free text as written) */
function answerNames(q: Question, answer: string | string[] | undefined): string[] {
  const values: string[] = answer === undefined ? [] : typeof answer === 'string' ? [answer] : answer;
  return values.map((v: string) => q.options.find((o: QuestionOption) => o.id === v)?.label ?? v);
}

/**
 * Questions.tsx QuestionRecord: an answered or skipped question keeps one row in the message, the answers folded under it
 * (open when answered)
 */
@ComponentV2
export struct QuestionRecord {
  @Param block: Block = { type: 'question' };
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;

  answers(q: Question): string[] {
    const all = this.block.answers;
    return answerNames(q, all !== undefined ? all[q.id] : undefined);
  }

  spec(): RowSpec {
    const n = this.block.answers !== undefined ? Object.keys(this.block.answers).length : 0;
    const s = new RowSpec();
    s.icon = 'messageCircleQuestion';
    s.height = this.rowH;
    s.label = this.block.outcome === 'answered' ? (n === 1 ? t('question.answeredOne') : tp('question.answered', 'n', `${n}`)) :
      this.block.outcome === 'skipped' ? t('question.skipped') : t('question.cancelled');
    return s;
  }

  build() {
    Disclosure({
      spec: this.spec(), foldKey: `${this.scope}:question:${this.block.id ?? ''}`,
      defaultOpen: this.block.outcome === 'answered'
    }) {
      Column({ space: WebMetrics.s1_5 }) {
        ForEach(this.block.questions ?? [], (q: Question) => {
          Column() {
            Text(q.text)
              .fontSize(WebType.text2)
              .lineHeight(WebType.text2Line)
              .fontColor($r('app.color.fg3'))
            Text(this.answers(q).length === 0 ? t('question.noAnswer') : this.answers(q).join(t('common.listSep')))
              .fontSize(WebType.text2)
              .lineHeight(WebType.text2Line)
              .fontColor(this.answers(q).length === 0 ? $r('app.color.fg3') : $r('app.color.fg1'))
          }
          .alignItems(HorizontalAlign.Start)
          .width('100%')
        }, (q: Question) => q.id)
      }
      .width('100%')
    }
  }
}

/** A block type this build does not know (an engine ahead of the app): a quiet row naming it */
@ComponentV2
export struct UnknownBlockRow {
  @Param type: string = '';
  @Param rowH: number = WebMetrics.chatRow;

  spec(): RowSpec {
    const s = new RowSpec();
    s.icon = 'puzzle';
    s.height = this.rowH;
    s.label = t('mobile.unknownBlock');
    s.target = this.type;
    s.targetMono = true;
    s.targetInk = $r('app.color.fg3');
    return s;
  }

  build() {
    AcpRow({ spec: this.spec() })
  }
}
