import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, SubagentSummary } from '../wire/Blocks';
import {
  firstPlainLine, isWaiting, ModelMark, SidekickModels, sidekickMark, stateLabel, statusLabel, subagentElapsed
} from '../logic/Subagents';
import { commandDuration, lastComponent, toolVerb, tp } from '../logic/Folding';
import { t } from '../store/L10n';
import { AgentMark } from './Glyphs';
import { AcpRow, RowSpec, ticker } from './Rows';

// apps/ios Transcript/SidekickRelay.swift (chat/subagents/SidekickRelay.tsx), after apps/android
// ui/subagents/SidekickRelay.kt: a round of the session's sidekick in the transcript. The handoff row names the
// sidekick's model taking the task, its latest steps sit in a fixed-height window, and the handback lands in the
// window's last slot. Nothing changes height while the round runs. The relay stands still: this transcript plays no
// entrances, so the other clients' flip, light and push are left out, and the marks keep the neutral ink as Android's.

/** One slot of the window: a step, or the handback once the round ended */
class RelayItem {
  key: string = '';
  step: Block | undefined = undefined;
}

/** What a rebuilt step row would show differently */
function stepSig(b: Block): string {
  return `${b.id ?? ''}:${b.status ?? ''}:${b.observation ?? ''}:${(b.target ?? '').length}:${b.startedAt ?? ''}:${b.endedAt ?? ''}`;
}

/**
 * One of the sidekick's tool rows, its mark in the lead slot. Rows stay plain: a command ends with ✓ / ✗ and its time,
 * another step only with a failure
 */
function stepSpec(b: Block, mark: ModelMark, now: () => number): RowSpec {
  const s = new RowSpec();
  s.lead = 'mark';
  s.mark = mark.brand ?? '';
  s.markName = mark.name;
  s.height = WebMetrics.processRow;
  s.label = toolVerb(b);
  s.shimmer = b.status === 'in_progress' && b.observation !== 'unknown';
  s.now = now;
  const target = (b.target ?? '').trim();
  // A read names its file: the window is narrow, and the path's head is the same for every step
  s.target = b.kind === 'read' ? lastComponent(target) : target;
  s.targetMono = b.targetMono === true;
  if (b.kind === 'execute') {
    s.outcomeSlot = true;
    s.outcome = b.status === 'completed' || b.status === 'failed' ? (b.status ?? '') : '';
    if (b.startedAt !== undefined && b.endedAt !== undefined) {
      s.duration = commandDuration(b.endedAt - b.startedAt);
    } else if (b.status === 'in_progress' && b.startedAt !== undefined) {
      s.runSince = b.startedAt;
    }
  } else if (b.status === 'failed') {
    s.outcome = 'failed';
  }
  return s;
}

@ComponentV2
export struct SidekickRelay {
  @Param node: SubagentSummary = { id: '', turnIndex: 0, state: 'running', announcedAt: 0 };
  @Param models: SidekickModels = new SidekickModels();
  @Param now: () => number = () => Date.now();

  model(): string {
    return this.models.name(this.node) ?? t('subagents.untitled');
  }

  mark(): ModelMark {
    return sidekickMark(this.node, this.model());
  }

  running(): boolean {
    return this.node.state === 'running';
  }

  items(): RelayItem[] {
    const out = (this.node.steps ?? []).map((b: Block) => {
      const item = new RelayItem();
      item.key = stepSig(b);
      item.step = b;
      return item;
    });
    if (!this.running()) {
      const back = new RelayItem();
      back.key = `\u0000back:${this.node.state}:${(this.node.result ?? '').length}:${this.node.toolCount ?? 0}:${this.node.endedAt ?? ''}`;
      out.push(back);
    }
    return out;
  }

  /**
   * A running round holds the whole window, so its steps never push what sits below (the lead working in parallel, or
   * its words already written); the slots it left unused close with the handback, so a short round leaves no gap
   */
  slots(count: number): number {
    return this.running() ? WebMetrics.sidekickSlots : Math.max(1, Math.min(WebMetrics.sidekickSlots, count));
  }

  build() {
    Column({ space: WebMetrics.processRowGap }) {
      RelayHead({ node: this.node, model: this.model(), mark: this.mark(), away: this.models.away(this.node), now: this.now })
      RelayWindow({ node: this.node, items: this.items(), slots: this.slots(this.items().length), mark: this.mark(), now: this.now })
    }
    .alignItems(HorizontalAlign.Start)
    .accessibilityText(tp('sidekick.relay', 'model', this.model()))
    .width('100%')
  }
}

/**
 * A CLI's logo on a page-coloured tile (subagents.css `.subagent-harness-mark`): the corner badge of a sidekick's mark
 * when it runs in another CLI. The caller places it
 */
@ComponentV2
struct HarnessBadge {
  @Param agent: string = '';

  build() {
    Stack() {
      AgentMark({ agent: this.agent, name: this.agent, tint: $r('app.color.fg1'), extent: WebMetrics.subagentBadge - 2 * Metrics.hairline })
    }
    .width(WebMetrics.subagentBadge)
    .height(WebMetrics.subagentBadge)
    .borderRadius(Metrics.rSm)
    .backgroundColor($r('app.color.bg0'))
  }
}

/**
 * The handoff row: the sidekick's mark (a CLI badge on its corner when it runs in another one), "<model> takes <task>"
 * from the second round on with its number, then the status and the clock
 */
@ComponentV2
struct RelayHead {
  @Param node: SubagentSummary = { id: '', turnIndex: 0, state: 'running', announcedAt: 0 };
  @Param model: string = '';
  @Param mark: ModelMark = new ModelMark();
  @Param away: string | undefined = undefined;
  @Param now: () => number = () => Date.now();
  /** Squeezed below two characters the task leaves instead of showing a bare ellipsis (SqueezedTitle) */
  @Local room: boolean = true;

  aboutToAppear(): void {
    ticker.start();
  }

  elapsed(): string {
    // Reads the shared tick, so a running round's time follows the clock
    const tick = ticker.tick;
    return tick >= 0 ? subagentElapsed(this.node, this.now()) : '';
  }

  status(): string {
    return isWaiting(this.node) ? statusLabel(this.node) : this.node.state === 'running' ? stateLabel(this.node) : '';
  }

  round(): number {
    return this.node.harness?.round ?? 1;
  }

  build() {
    Row() {
      Stack() {
        AgentMark({ agent: this.mark.brand ?? '', name: this.mark.name, tint: $r('app.color.fg1'), extent: WebMetrics.icon })
        if (this.away !== undefined) {
          // Where a summoned child's row carries it: its mark slot is `subagentMark` wide around the same mark
          HarnessBadge({ agent: this.away })
            .position({
              x: (WebMetrics.lead + WebMetrics.icon) / 2 + (WebMetrics.subagentMark - WebMetrics.icon) / 2 -
                WebMetrics.subagentBadgeInset - WebMetrics.subagentBadge,
              y: (WebMetrics.lead + WebMetrics.icon) / 2 + (WebMetrics.subagentMark - WebMetrics.icon) / 2 -
                WebMetrics.subagentBadgeInset - WebMetrics.subagentBadge
            })
        }
      }
      .width(WebMetrics.lead)
      .height(WebMetrics.lead)
      .margin({ left: (WebMetrics.icon - WebMetrics.lead) / 2, right: WebMetrics.rowGap - (WebMetrics.lead - WebMetrics.icon) / 2 })
      // The task gives way first, then the model name; the verb and the round stay. Plain Texts, as a custom component's
      // flexShrink does not reach its Text
      Row({ space: Metrics.s2 }) {
        Row({ space: Metrics.s2 }) {
          Text(this.model)
            .fontSize(WebType.text2)
            .lineHeight(WebType.text2Line)
            .fontColor($r('app.color.fg1'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .flexShrink(1)
          Text(t('sidekick.takes'))
            .fontSize(WebType.text2)
            .lineHeight(WebType.text2Line)
            .fontColor($r('app.color.fg2'))
            .maxLines(1)
            .flexShrink(0)
          if (this.round() > 1) {
            Text(tp('subagents.round', 'n', `${this.round()}`))
              .fontSize(WebType.text3)
              .lineHeight(WebType.text3Line)
              .fontColor($r('app.color.fg3'))
              .maxLines(1)
              .flexShrink(0)
          }
        }
        .constraintSize({ maxWidth: '100%' })
        if ((this.node.title ?? '').length > 0) {
          Text(this.node.title ?? '')
            .fontSize(WebType.text2)
            .lineHeight(WebType.text2Line)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .layoutWeight(1)
            .opacity(this.room ? 1 : 0)
            .onAreaChange((before: Area, after: Area) => {
              this.room = Number(after.width) >= WebType.text2 * 2;
            })
        }
      }
      .layoutWeight(1)
      .clip(true)
      Row({ space: Metrics.s1 }) {
        if (this.status().length > 0) {
          Text(this.status())
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor(isWaiting(this.node) ? $r('app.color.warn') : $r('app.color.fg3'))
            .maxLines(1)
        }
        Text(this.elapsed())
          .fontSize(WebType.text3)
          .lineHeight(WebType.text3Line)
          .fontColor($r('app.color.fg3'))
          .fontFeature('"tnum" on')
          .maxLines(1)
      }
      .margin({ left: Metrics.gap })
      .flexShrink(0)
    }
    .alignItems(VerticalAlign.Center)
    .constraintSize({ minHeight: WebMetrics.processRow })
    .width('100%')
  }
}

/**
 * The fixed-height window: the newest steps, then the handback. Once the slots overflow the rows sit on the window's
 * bottom edge and the top one fades out
 */
@ComponentV2
struct RelayWindow {
  @Param node: SubagentSummary = { id: '', turnIndex: 0, state: 'running', announcedAt: 0 };
  @Param items: RelayItem[] = [];
  @Param slots: number = 1;
  @Param mark: ModelMark = new ModelMark();
  @Param now: () => number = () => Date.now();

  shown(): RelayItem[] {
    return this.items.slice(Math.max(0, this.items.length - this.slots));
  }

  /** The round's end: what came back, or how it stopped, then its step count and time */
  handback(): RowSpec {
    const ok = this.node.state === 'completed';
    const s = new RowSpec();
    s.lead = 'mark';
    s.mark = this.mark.brand ?? '';
    s.markName = this.mark.name;
    s.height = WebMetrics.processRow;
    s.label = ok ? t('sidekick.handback') : stateLabel(this.node);
    s.labelInk = ok ? $r('app.color.fg2') : this.node.state === 'failed' ? $r('app.color.danger') : $r('app.color.fg3');
    s.target = firstPlainLine(this.node.result ?? '') ?? '';
    s.targetInk = $r('app.color.fg1');
    s.meta = tp('sidekick.steps', 'n', `${this.node.toolCount ?? 0}`);
    s.duration = subagentElapsed(this.node, this.now());
    return s;
  }

  extent(): number {
    return this.slots * WebMetrics.processRow + (this.slots - 1) * WebMetrics.processRowGap;
  }

  more(): boolean {
    return this.items.length > this.slots;
  }

  build() {
    Stack({ alignContent: Alignment.Top }) {
      Column({ space: WebMetrics.processRowGap }) {
        ForEach(this.shown(), (item: RelayItem) => {
          if (item.step !== undefined) {
            AcpRow({ spec: stepSpec(item.step, this.mark, this.now) })
          } else {
            AcpRow({ spec: this.handback() })
          }
        }, (item: RelayItem) => item.key)
      }
      .width('100%')
      if (this.more()) {
        // A mask, not a wash of the page colour: it keeps the rows' alpha from clear at the top edge to whole a row down,
        // whatever lies behind the transcript
        Column()
          .width('100%')
          .height('100%')
          .linearGradient({
            angle: 180,
            colors: [[Color.Transparent, 0], [Color.Black, WebMetrics.processRow * WebMetrics.sidekickFadeRows / this.extent()]]
          })
          .blendMode(BlendMode.DST_IN)
          .hitTestBehavior(HitTestMode.None)
      }
    }
    .height(this.extent())
    .width('100%')
    .clip(true)
    .blendMode(BlendMode.SRC_OVER, this.more() ? BlendApplyType.OFFSCREEN : BlendApplyType.FAST)
  }
}
