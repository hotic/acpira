import { LengthMetrics } from '@kit.ArkUI';
import { AgentBlock, PermissionOption, Turn } from '../wire/Wire';
import { Metrics, Type } from '../design/Tokens';
import { t } from '../store/L10n';
import { Lucide } from './Glyphs';
import { Prose } from './Prose';

// One turn of the thread (apps/ios Transcript: UserMessage, AgentMessage, CodexFold, ProcessViews, PermissionCard):
// the prompt as a right-aligned bubble; a reply as its process folded into one row (thoughts, tool calls, commentary),
// then the reply text and any permission card.

/** The icon a tool row leads with, by the call's kind (ToolCall.tsx toolIcon) */
function toolIcon(kind: string | undefined): string {
  switch (kind) {
    case 'read':
      return 'eye';
    case 'edit':
    case 'delete':
    case 'move':
      return 'pencil';
    case 'execute':
      return 'terminal';
    case 'search':
      return 'search';
    case 'fetch':
      return 'globe';
    case 'think':
      return 'brain';
    default:
      return 'wrench';
  }
}

/** The blocks a turn folds away, and the reply after them: text after the last process block is the reply */
class TurnSplit {
  process: AgentBlock[] = [];
  reply: AgentBlock[] = [];
  permissions: AgentBlock[] = [];
}

function splitTurn(blocks: AgentBlock[]): TurnSplit {
  const split = new TurnSplit();
  let lastProcess = -1;
  blocks.forEach((b: AgentBlock, i: number) => {
    if (b.type === 'tool_call' || b.type === 'thought' || b.type === 'plan' || b.type === 'compaction') {
      lastProcess = i;
    }
  });
  blocks.forEach((b: AgentBlock, i: number) => {
    if (b.type === 'permission') {
      split.permissions.push(b);
    } else if (b.type === 'text' && i > lastProcess) {
      split.reply.push(b);
    } else if (b.type === 'tool_call' || b.type === 'thought' || b.type === 'text') {
      split.process.push(b);
    }
  });
  return split;
}

function firstLine(text: string | undefined): string {
  return (text ?? '').split('\n').find((l: string) => l.trim().length > 0)?.trim() ?? '';
}

@ComponentV2
export struct UserBubble {
  @Param text: string = '';

  build() {
    Row() {
      Text(this.text)
        .fontSize(Type.body)
        .lineHeight(Type.bodyLine)
        .fontColor($r('app.color.fg1'))
        .copyOption(CopyOptions.LocalDevice)
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2 + Metrics.s1, bottom: Metrics.s2 + Metrics.s1 })
        .backgroundColor($r('app.color.bubble_bg'))
        .borderRadius(Metrics.rBubble)
        .constraintSize({ maxWidth: '100%' })
    }
    .justifyContent(FlexAlign.End)
    .padding({ left: Metrics.bubbleInset })
    .width('100%')
  }
}

/** One process row: the tool's icon, its verb and target, on one line (ProcessViews.swift ToolRow) */
@ComponentV2
struct ProcessRow {
  @Param block: AgentBlock = { type: 'text' };

  build() {
    Row({ space: Metrics.s2 }) {
      if (this.block.type === 'thought') {
        Lucide({ name: 'brain', tint: $r('app.color.fg3'), extent: Metrics.icon })
        Text(firstLine(this.block.text).length > 0 ? firstLine(this.block.text) : t('host.thinking'))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg2'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .layoutWeight(1)
      } else if (this.block.type === 'text') {
        Text(firstLine(this.block.markdown))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg2'))
          .maxLines(2)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .layoutWeight(1)
          .padding({ left: Metrics.lead + Metrics.s2 })
      } else {
        if (this.block.status === 'in_progress' || this.block.status === 'pending') {
          LoadingProgress().color($r('app.color.fg3')).width(Metrics.icon).height(Metrics.icon)
        } else if (this.block.status === 'failed') {
          Lucide({ name: 'circleAlert', tint: $r('app.color.danger'), extent: Metrics.icon })
        } else {
          Lucide({ name: toolIcon(this.block.kind), tint: $r('app.color.fg3'), extent: Metrics.icon })
        }
        Text() {
          Span(this.block.verb ?? '').fontColor($r('app.color.fg2'))
          Span(this.block.target !== undefined ? `  ${this.block.target}` : '').fontColor($r('app.color.fg3'))
        }
        .fontSize(Type.text3)
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      }
    }
    .height(Metrics.chatRow)
    .width('100%')
  }
}

/** A permission the agent waits on: what it wants to do, and its options as buttons (PermissionCard.swift) */
@ComponentV2
struct PermissionCard {
  @Param block: AgentBlock = { type: 'permission' };
  @Event onPick: (blockId: string, optionId: string) => void = () => {};

  build() {
    Column({ space: Metrics.gap }) {
      Text(this.block.title ?? '')
        .fontSize(Type.text2)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg1'))
      if (this.block.command !== undefined) {
        Text(this.block.command)
          .fontFamily('monospace')
          .fontSize(Type.mono)
          .lineHeight(Type.monoLine)
          .fontColor($r('app.color.fg1'))
          .padding(Metrics.gap)
          .backgroundColor($r('app.color.bg_code'))
          .borderRadius(Metrics.rMd)
          .width('100%')
      }
      Flex({ wrap: FlexWrap.Wrap, space: { main: LengthMetrics.vp(Metrics.s2), cross: LengthMetrics.vp(Metrics.s2) } }) {
        ForEach(this.block.options ?? [], (o: PermissionOption, i: number) => {
          Button(o.label, { type: ButtonType.Capsule })
            .fontSize(Type.text3)
            .fontColor(i === 0 ? $r('app.color.btn1_fg') : $r('app.color.fg1'))
            .backgroundColor(i === 0 ? $r('app.color.btn1_bg') : $r('app.color.chip'))
            .height(Metrics.ctl)
            .padding({ left: Metrics.pad, right: Metrics.pad })
            .id(i === 0 ? 'permission.allow' : `permission.${o.id}`)
            .onClick(() => this.onPick(this.block.id ?? '', o.id))
        }, (o: PermissionOption) => o.id)
      }
    }
    .alignItems(HorizontalAlign.Start)
    .padding(Metrics.pad)
    .backgroundColor($r('app.color.card'))
    .borderRadius(Metrics.rLg)
    .border({ width: Metrics.hairline, color: $r('app.color.line') })
    .width('100%')
  }
}

@ComponentV2
export struct AgentMessage {
  @Param turn: Turn = { role: 'agent' };
  @Param running: boolean = false;
  @Event onPermission: (blockId: string, optionId: string) => void = () => {};
  @Local open: boolean = false;

  build() {
    Column({ space: Metrics.gap }) {
      if (splitTurn(this.turn.blocks ?? []).process.length > 0 || this.running) {
        Column() {
          // The fold's head: what the turn is doing, or that it is done and how long it took
          Row({ space: Metrics.s2 }) {
            if (this.running) {
              LoadingProgress().color($r('app.color.fg3')).width(Metrics.icon).height(Metrics.icon)
              Text(t('host.working')).fontSize(Type.text3).fontColor($r('app.color.fg2'))
            } else {
              Lucide({ name: 'check', tint: $r('app.color.fg3'), extent: Metrics.icon })
              Text(t('turns.done')).fontSize(Type.text3).fontColor($r('app.color.fg2'))
              if (this.turn.startedAt !== undefined && this.turn.endedAt !== undefined) {
                Text(`${Math.max(1, Math.round((this.turn.endedAt - this.turn.startedAt) / 1000))}s`)
                  .fontSize(Type.text3)
                  .fontColor($r('app.color.fg3'))
              }
              Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: Metrics.icon - Metrics.s1 })
                .rotate({ angle: this.open ? 90 : 0 })
            }
          }
          .height(Metrics.chatRow)
          .width('100%')
          .onClick(() => {
            this.open = !this.open;
          })

          if (this.open || this.running) {
            // While the turn runs only its latest step shows under the head
            ForEach(this.open ? splitTurn(this.turn.blocks ?? []).process : splitTurn(this.turn.blocks ?? []).process.slice(-1),
              (b: AgentBlock) => {
                ProcessRow({ block: b })
              }, (b: AgentBlock, i: number) => `${b.id ?? i}:${b.status ?? ''}`)
          }
        }
        .width('100%')
      }
      ForEach(splitTurn(this.turn.blocks ?? []).reply, (b: AgentBlock) => {
        Prose({ markdown: b.markdown ?? '' })
      }, (b: AgentBlock, i: number) => `${b.id ?? i}:${(b.markdown ?? '').length}`)
      ForEach(splitTurn(this.turn.blocks ?? []).permissions, (b: AgentBlock) => {
        PermissionCard({ block: b, onPick: (blockId: string, optionId: string) => this.onPermission(blockId, optionId) })
      }, (b: AgentBlock, i: number) => b.id ?? `${i}`)
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}
