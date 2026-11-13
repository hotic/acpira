import { LengthMetrics } from '@kit.ArkUI';
import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, PermissionChoice, TurnX } from '../wire/Blocks';
import {
  clockStart, compactTokens, DisplayOption, durationText, elapsedLabel, permissionOption, planApprovalTitle, planHeading,
  reviseOption, splitCodexBlocks
} from '../logic/Folding';
import { t } from '../store/L10n';
import { copyText } from './Copy';
import { AgentMark, Lucide } from './Glyphs';
import { Prose } from './Prose';
import { captureShare, shareBlocks, shareMeta, SharePayload, SharePreview } from './ShareCard';
import { image } from '@kit.ImageKit';

// The transcript's cards (apps/ios Transcript/Cards.swift PermissionCard + TurnActions, PlanDocumentCard.swift; apps/android
// ui/Transcript.kt PermissionCard / TurnActions, ui/Cards.kt PlanDocumentCard): a permission request with every option,
// a saved plan with its Build bar, and the row of actions under a finished reply.

// Buttons

/**
 * Components/Buttons.swift AcpButtonStyle: a capsule with a hairline, text-2; the primary answer takes the active fill
 * and the strong ink. `oneLine`: a menu trigger, exactly --ctl tall; otherwise the answer form, whose label may wrap
 */
@ComponentV2
export struct PillButton {
  @Param label: string = '';
  @Param primary: boolean = false;
  @Param oneLine: boolean = false;
  /** A trailing chevron (a menu trigger) */
  @Param chevron: boolean = false;
  @Event onTap: () => void = () => {};

  build() {
    Row({ space: WebMetrics.s1_5 }) {
      Text(this.label)
        .fontSize(WebType.text2)
        .lineHeight(WebType.text2Line)
        .fontWeight(this.primary ? FontWeight.Medium : FontWeight.Normal)
        .fontColor(this.primary ? $r('app.color.fg_strong') : $r('app.color.fg2'))
        .maxLines(this.oneLine ? 1 : WebMetrics.allLines)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .flexShrink(1)
      if (this.chevron) {
        Lucide({ name: 'chevronDown', tint: $r('app.color.fg2'), extent: WebMetrics.chevron, weight: WebMetrics.strokeChevron })
      }
    }
    .padding({
      left: Metrics.s3 + Metrics.hairline, right: Metrics.s3 + Metrics.hairline,
      top: this.oneLine ? 0 : Metrics.gapHalf + Metrics.hairline, bottom: this.oneLine ? 0 : Metrics.gapHalf + Metrics.hairline
    })
    .constraintSize({ minHeight: Metrics.ctl })
    .backgroundColor(this.primary ? $r('app.color.active') : Color.Transparent)
    .border({ width: Metrics.hairline, color: this.primary ? $r('app.color.line_strong') : $r('app.color.line') })
    .borderRadius(Metrics.ctl / 2)
    .onClick(() => this.onTap())
  }
}

/**
 * A card's action (CardButton): a capsule; the way on is prominent in the send button's ink, and falls back to the plain
 * fill with faint text while it cannot run. One line, never wrapped
 */
@ComponentV2
export struct CardButton {
  @Param label: string = '';
  @Param primary: boolean = false;
  @Param active: boolean = true;
  /** An agent's mark before the label (the plan's execution model), and a menu chevron after it */
  @Param mark: string = '';
  @Param chevron: boolean = false;
  @Event onTap: () => void = () => {};

  build() {
    Row({ space: WebMetrics.s1_5 }) {
      if (this.mark.length > 0) {
        AgentMark({ agent: this.mark, tint: $r('app.color.fg1'), extent: Metrics.icon })
          .flexShrink(0)
      }
      Text(this.label)
        .fontSize(WebType.text2)
        .fontWeight(this.primary && this.active ? FontWeight.Medium : FontWeight.Normal)
        .fontColor(this.primary && this.active ? $r('app.color.bg0') : (this.active ? $r('app.color.fg1') : $r('app.color.pop_fg3')))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .flexShrink(1)
      if (this.chevron) {
        Lucide({ name: 'chevronDown', tint: $r('app.color.pop_fg3'), extent: WebMetrics.chevron, weight: WebMetrics.strokeChevron })
          .flexShrink(0)
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .height(Metrics.ctl)
    .backgroundColor(this.primary && this.active ? $r('app.color.fg_strong') : $r('app.color.active'))
    .borderRadius(Metrics.ctl / 2)
    .onClick(() => {
      if (this.active) {
        this.onTap();
      }
    })
  }
}

/**
 * CardSurface: the composer bar's own material and corners (bg-1, a hairline, the card radius and the bar's shadow), so a
 * card in the thread reads as the same thing as the bar under it
 */
@Extend(Column)
function cardSurface() {
  .backgroundColor($r('app.color.bg1'))
  .border({ width: Metrics.hairline, color: $r('app.color.line') })
  .borderRadius(WebMetrics.rCard)
  .shadow({ radius: WebMetrics.popShadow, color: $r('app.color.hover') })
  .width('100%')
}

// Permission

/** The everyday grants first, then the ones that bypass approvals */
function menuOrder(options: DisplayOption[]): DisplayOption[] {
  return options.filter((o: DisplayOption) => !o.bypass).concat(options.filter((o: DisplayOption) => o.bypass));
}

/**
 * A permission request (Cards.swift PermissionCard): the request's title and command on a card, Allow once as the
 * filled action, Reject beside it, the other grants behind More options. Several allow-once answers and no
 * allow-always make a list of answers instead, every option an equal button
 */
@ComponentV2
export struct PermissionCard {
  @Param block: Block = { type: 'permission' };
  @Event onPick: (optionId: string) => void = (optionId: string) => {};

  options(): DisplayOption[] {
    return (this.block.options ?? []).map((o: PermissionChoice) => permissionOption(o, this.block.planId !== undefined));
  }

  flat(): boolean {
    const o = this.options();
    return !o.some((x: DisplayOption) => x.kind === 'allow_always') && o.filter((x: DisplayOption) => x.kind === 'allow_once').length > 1;
  }

  once(): DisplayOption | undefined {
    return this.flat() ? undefined : this.options().find((o: DisplayOption) => o.kind === 'allow_once');
  }

  reject(): DisplayOption | undefined {
    return this.flat() ? undefined : this.options().find((o: DisplayOption) => o.kind === 'reject_once');
  }

  more(): DisplayOption[] {
    if (this.flat()) {
      return [];
    }
    const once = this.once();
    const reject = this.reject();
    return menuOrder(this.options().filter((o: DisplayOption) => o.id !== once?.id && o.id !== reject?.id));
  }

  @Builder
  moreMenu() {
    Menu() {
      ForEach(this.more(), (o: DisplayOption) => {
        MenuItem({ content: o.label, labelInfo: o.detail })
          .onClick(() => this.onPick(o.id))
      }, (o: DisplayOption) => o.id)
    }
  }

  build() {
    Column({ space: Metrics.gap }) {
      Row({ space: Metrics.gap }) {
        if (this.block.command !== undefined) {
          Lucide({ name: 'terminal', tint: $r('app.color.fg2'), extent: Metrics.icon })
        }
        Text(this.block.command !== undefined ? t('permission.run') : planApprovalTitle(this.block.title ?? ''))
          .fontSize(WebType.text2)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg2'))
          .maxLines(2)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .flexShrink(1)
      }
      if (this.block.command !== undefined) {
        Text(this.block.command)
          .fontFamily('monospace')
          .fontSize(WebType.mono)
          .lineHeight(WebType.monoLine)
          .fontColor($r('app.color.fg1'))
          .copyOption(CopyOptions.LocalDevice)
      }
      if ((this.block.description ?? '').trim().length > 0) {
        Text(this.block.description)
          .fontSize(WebType.text3)
          .lineHeight(WebType.text3Line)
          .fontColor($r('app.color.fg2'))
      }
      if (this.flat()) {
        Flex({ wrap: FlexWrap.Wrap, space: { main: LengthMetrics.vp(Metrics.gap), cross: LengthMetrics.vp(Metrics.gap) } }) {
          ForEach(this.options(), (o: DisplayOption) => {
            CardButton({ label: o.label, onTap: () => this.onPick(o.id) })
          }, (o: DisplayOption) => o.id)
        }
      } else {
        // The extra grants left at their whole width, the answers pushed right; answers that do not fit side by side
        // wrap as whole buttons (iOS FlowLayout), so no label is cut
        Row({ space: Metrics.gap }) {
          if (this.more().length > 0) {
            CardButton({ label: t('permission.more'), chevron: true })
              .flexShrink(0)
              .bindMenu(this.moreMenu())
          }
          Flex({
            wrap: FlexWrap.Wrap, justifyContent: FlexAlign.End,
            space: { main: LengthMetrics.vp(Metrics.gap), cross: LengthMetrics.vp(Metrics.gap) }
          }) {
            if (this.reject() !== undefined) {
              CardButton({
                label: this.reject()?.label ?? '', primary: this.block.defaultToNo === true,
                onTap: () => this.onPick(this.reject()?.id ?? '')
              })
            }
            if (this.once() !== undefined) {
              CardButton({
                label: this.once()?.label ?? '', primary: this.block.defaultToNo !== true,
                onTap: () => this.onPick(this.once()?.id ?? '')
              })
            }
          }
          .layoutWeight(1)
        }
        .alignItems(VerticalAlign.Top)
        .width('100%')
      }
    }
    .alignItems(HorizontalAlign.Start)
    .padding(Metrics.pad)
    .cardSurface()
  }
}

// Plan document

/** The session's model control, read loosely from the view (transcript.ts SessionControls / ConfigControl) */
interface PlanModelOption {
  id: string;
  name: string;
}

interface PlanConfig {
  id: string;
  category?: string;
  value?: string;
  options?: PlanModelOption[];
}

interface PlanControls {
  options?: PlanConfig[];
}

/** The execution model the plan card offers: the session's model control, its options and the current pick */
export class PlanExecutor {
  configId: string = '';
  value: string = '';
  options: PlanModelOption[] = [];

  label(): string {
    return this.options.find((o: PlanModelOption) => o.id === this.value)?.name ?? this.value;
  }
}

export function planExecutor(controls: Object | undefined, picked: string): PlanExecutor | undefined {
  const config = ((controls as PlanControls | undefined)?.options ?? []).find((c: PlanConfig) => c.category === 'model');
  if (config === undefined || (config.options ?? []).length === 0) {
    return undefined;
  }
  const e = new PlanExecutor();
  e.configId = config.id;
  e.value = picked.length > 0 ? picked : (config.value ?? '');
  e.options = config.options ?? [];
  return e;
}

/**
 * A saved plan as its own card between the work before it and the continuation after it (PlanDocumentCard.swift). The
 * card is the plan itself, its opening faded at a cap with the whole plan one tap away (a sheet with a copy), then an
 * action bar while it can still be built: the execution model with the extra approvals behind one chip, and Build
 */
@ComponentV2
export struct PlanDocumentCard {
  @Param block: Block = { type: 'plan_document' };
  /** The approval the agent waits on for this plan, when it asks for one */
  @Param permission: Block | undefined = undefined;
  @Param running: boolean = false;
  /** The view's controls (the model control) and agent (the chip's mark) */
  @Param controls: Object | undefined = undefined;
  @Param agent: string = '';
  @Event onBuild: (optionId: string, configId: string, value: string) => void =
    (optionId: string, configId: string, value: string) => {};
  @Event onPermission: (blockId: string, optionId: string) => void = (blockId: string, optionId: string) => {};
  @Local natural: number = 0;
  @Local sheet: boolean = false;
  @Local picked: string = '';

  title(): string {
    return planHeading(this.block)[0];
  }

  body(): string {
    return planHeading(this.block)[1];
  }

  executor(): PlanExecutor | undefined {
    return planExecutor(this.controls, this.picked);
  }

  primary(): PermissionChoice | undefined {
    return (this.permission?.options ?? []).find((o: PermissionChoice) => o.kind === 'allow_once');
  }

  extra(): PermissionChoice[] {
    const primary = this.primary();
    const revise = reviseOption(this.permission?.options ?? []);
    return (this.permission?.options ?? []).filter((o: PermissionChoice) => o.id !== primary?.id && o.id !== revise?.id);
  }

  canBuild(): boolean {
    const busy = (this.running && this.permission === undefined) || this.block.status === 'draft' ||
      this.block.status === 'executing';
    return !busy && (this.block.markdown ?? '').length > 0;
  }

  build_(optionId: string): void {
    const e = this.executor();
    this.onBuild(optionId, e?.configId ?? '', e?.value ?? '');
  }

  pick(o: PermissionChoice): void {
    if (o.kind === 'allow_once' || o.kind === 'allow_always') {
      this.build_(o.id);
    } else if (this.permission !== undefined) {
      this.onPermission(this.permission.id ?? '', o.id);
    }
  }

  @Builder
  executorMenu() {
    Menu() {
      if (this.executor() !== undefined) {
        MenuItemGroup({ header: t('plan.executor') }) {
          ForEach(this.executor()?.options ?? [], (o: PlanModelOption) => {
            MenuItem({ content: o.name })
              .selected(o.id === this.executor()?.value)
              .selectIcon(true)
              .onClick(() => {
                this.picked = o.id;
              })
          }, (o: PlanModelOption) => o.id)
        }
      }
      if (this.extra().length > 0) {
        MenuItemGroup({ header: t('plan.moreApprovals') }) {
          ForEach(this.extra(), (o: PermissionChoice) => {
            MenuItem({ content: permissionOption(o, true).label })
              .enabled(o.kind === 'allow_once' || o.kind === 'allow_always' ? this.canBuild() : true)
              .onClick(() => this.pick(o))
          }, (o: PermissionChoice) => o.id)
        }
      }
    }
  }

  @Builder
  wholePlan() {
    Column() {
      Row({ space: Metrics.s2 }) {
        Text(this.title())
          .fontSize(WebType.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .layoutWeight(1)
        Stack() {
          Lucide({ name: 'copy', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .width(Metrics.barCtl)
        .height(Metrics.barCtl)
        .accessibilityText(t('mobile.plan.copy'))
        .onClick(() => copyText(this.getUIContext(), this.block.markdown ?? ''))
      }
      .padding({ left: Metrics.pad, right: Metrics.s1 })
      .width('100%')
      Scroll() {
        Prose({ markdown: this.body() })
          .padding(Metrics.pad)
      }
      .scrollBar(BarState.Off)
      .align(Alignment.TopStart)
      .layoutWeight(1)
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }

  build() {
    Column() {
      if (this.body().length > 0) {
        Stack({ alignContent: Alignment.Bottom }) {
          // Unbounded inside, so the plan's own height can be read; the card shows its opening
          Scroll() {
            Column() {
              Prose({ markdown: this.body() })
            }
            .onAreaChange((before: Area, after: Area) => {
              this.natural = Number(after.height);
            })
          }
          .enableScrollInteraction(false)
          .scrollBar(BarState.Off)
          .align(Alignment.TopStart)
          .constraintSize({ maxHeight: WebMetrics.planBody })
          .width('100%')
          if (this.natural > WebMetrics.planBody) {
            // The cut fades the plan into the card, and the way to the rest sits in the room the fade leaves clear
            Column() {
              Column()
                .height(WebMetrics.scrollFade)
                .width('100%')
                .linearGradient({ angle: 180, colors: [[Color.Transparent, 0], [$r('app.color.bg1'), 1]] })
              Text(t('plan.expand'))
                .fontSize(WebType.text3)
                .lineHeight(WebType.text3Line)
                .fontColor($r('app.color.pop_fg2'))
                .textAlign(TextAlign.Center)
                .padding({ top: Metrics.s1 })
                .backgroundColor($r('app.color.bg1'))
                .width('100%')
            }
            .width('100%')
          }
        }
        .padding(Metrics.pad)
        .width('100%')
        .onClick(() => {
          this.sheet = true;
        })
        .bindSheet(this.sheet, this.wholePlan(), {
          height: SheetSize.LARGE, dragBar: true, showClose: false, onDisappear: () => {
            this.sheet = false;
          }
        })
      }
      if (this.block.status !== 'approved' && this.block.status !== 'executing') {
        Row({ space: Metrics.gap }) {
          if (this.executor() !== undefined || this.extra().length > 0) {
            CardButton({
              label: this.executor()?.label() ?? t('plan.approvals'), mark: this.executor() !== undefined ? this.agent : '',
              chevron: true
            })
              .flexShrink(1)
              .bindMenu(this.executorMenu())
          }
          CardButton({
            label: this.primary() !== undefined ? permissionOption(this.primary() as PermissionChoice, true).label : t('plan.build'),
            primary: true, active: this.canBuild(), onTap: () => this.build_(this.primary()?.id ?? '')
          })
            .flexShrink(0)
        }
        .justifyContent(FlexAlign.End)
        .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.pad })
        .width('100%')
      }
    }
    .cardSurface()
    .clip(true)
  }
}

// Turn actions

/** The text a copy takes: the turn's final prose, else its plain text blocks */
function replyMarkdown(turn: TurnX): string {
  const blocks = (turn.blocks ?? []) as Block[];
  const reply = splitCodexBlocks(blocks).reply.map((b: Block) => b.markdown ?? '').join('\n\n').trim();
  if (reply.length > 0) {
    return reply;
  }
  return blocks.filter((b: Block) => b.type === 'text').map((b: Block) => b.markdown ?? '').join('\n\n').trim();
}

/** A stat line of the statistics card */
class StatLine {
  label: string = '';
  value: string = '';
}

function statLine(label: string, value: string): StatLine {
  const l = new StatLine();
  l.label = label;
  l.value = value;
  return l;
}

/** The statistics card's timing tiles: the duration and the first token */
function statTiles(turn: TurnX): StatLine[] {
  const out: StatLine[] = [];
  const elapsed = elapsedLabel(turn);
  if (elapsed.length > 0) {
    out.push(statLine(t('stats.duration'), elapsed));
  }
  if (turn.startedAt !== undefined && turn.respondedAt !== undefined && turn.respondedAt > turn.startedAt) {
    out.push(statLine(t('stats.firstToken'), durationText(turn.respondedAt - turn.startedAt)));
  }
  return out;
}

function statTokens(turn: TurnX): StatLine[] {
  const u = turn.usage;
  const out: StatLine[] = [];
  if (u?.input !== undefined) {
    out.push(statLine(t('stats.input'), compactTokens(u.input)));
  }
  if (u?.output !== undefined) {
    out.push(statLine(t('stats.output'), compactTokens(u.output)));
  }
  if (u?.context !== undefined) {
    out.push(statLine(t('stats.contextShort'), `${compactTokens(u.context.used)} / ${compactTokens(u.context.size)}`));
  }
  return out;
}

/** One glyph of the action row; `open` marks the one whose card is showing */
@ComponentV2
struct ActionGlyph {
  @Param icon: string = 'copy';
  @Param label: string = '';
  @Param open: boolean = false;
  @Event onTap: () => void = () => {};

  build() {
    Stack() {
      Lucide({ name: this.icon, tint: this.open ? $r('app.color.fg1') : $r('app.color.fg2'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
    }
    .width(WebMetrics.ctlSm)
    .height(WebMetrics.ctlSm)
    .borderRadius(WebMetrics.ctlSm / 2)
    .backgroundColor(this.open ? $r('app.color.active') : Color.Transparent)
    .accessibilityText(this.label)
    .onClick(() => this.onTap())
  }
}

/**
 * The reply's actions under it (Cards.swift TurnActions), hugging the reply: retry (the newest reply, failed in a way
 * Retry answers), copy, its screenshot (components/ShareCard.ets), fork from this turn, the statistics
 */
@ComponentV2
export struct TurnActions {
  @Param turn: TurnX = { role: 'agent' };
  @Param retry: boolean = false;
  /** Fork and retry are offered only when the chat wires them */
  @Param actions: boolean = false;
  /** The screenshot's watermark: the session's title, agent and controls (the model control), read loosely */
  @Param title: string = '';
  @Param agent: string = '';
  @Param controls: Object | undefined = undefined;
  @Event onRetry: () => void = () => {};
  @Event onFork: () => void = () => {};
  @Local copied: boolean = false;
  @Local stats: boolean = false;
  /** The reply's screenshot while its preview is up */
  @Local shot: image.PixelMap | undefined = undefined;
  /** A screenshot is being drawn: a second tap waits for it */
  private sharing: boolean = false;
  /** The row's width, which with the message's sides is the thread's; read on a tap */
  private rowWidth: number = 0;

  private share(): void {
    if (this.sharing || this.rowWidth <= 0) {
      return;
    }
    const payload = new SharePayload();
    payload.meta = shareMeta(this.title, this.agent, this.controls, this.turn);
    payload.blocks = shareBlocks(this.turn);
    payload.cardWidth = Math.min(this.rowWidth + Metrics.pad * 2, Metrics.contentW);
    this.sharing = true;
    captureShare(this.getUIContext(), payload).then((picture: image.PixelMap | undefined) => {
      this.sharing = false;
      this.shot = picture;
    });
  }

  @Builder
  sharePreview() {
    SharePreview({
      picture: this.shot, onClose: () => {
        this.shot = undefined;
      }
    })
  }

  @Builder
  statsCard() {
    Column({ space: Metrics.s1 }) {
      Row({ space: Metrics.s3 }) {
        Text(t('turn.stats'))
          .fontSize(WebType.text2)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
        Text(this.turn.usage?.model ?? '')
          .fontSize(WebType.text3)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
            .ellipsisMode(EllipsisMode.CENTER)
          .textAlign(TextAlign.End)
          .layoutWeight(1)
      }
      .padding({ left: Metrics.s2, right: Metrics.s2 })
      .height(Metrics.ctl)
      .width('100%')
      if (statTiles(this.turn).length > 0) {
        Row() {
          ForEach(statTiles(this.turn), (tile: StatLine, i: number) => {
            Column({ space: WebMetrics.s0_5 }) {
              Text(tile.value)
                .fontSize(WebType.text2)
                .fontWeight(FontWeight.Medium)
                .fontColor($r('app.color.fg1'))
                .fontFeature('"tnum" on')
                .maxLines(1)
              Text(tile.label)
                .fontSize(WebType.text4)
                .lineHeight(WebType.text4Line)
                .fontColor($r('app.color.fg3'))
                .maxLines(1)
            }
            .alignItems(HorizontalAlign.Start)
            .padding({ left: Metrics.s2, right: Metrics.s2, top: WebMetrics.s1_5, bottom: WebMetrics.s1_5 })
            .border({ width: { left: i > 0 ? Metrics.hairline : 0 }, color: { left: $r('app.color.line') } })
            .layoutWeight(1)
          }, (tile: StatLine) => tile.label)
        }
        .margin({ left: WebMetrics.s1_5, right: WebMetrics.s1_5 })
        .backgroundColor($r('app.color.active'))
        .borderRadius(Metrics.rMd)
      }
      ForEach(statTokens(this.turn), (line: StatLine) => {
        Row({ space: Metrics.s2 }) {
          Text(line.label)
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor($r('app.color.fg2'))
            .maxLines(1)
            .layoutWeight(1)
          Text(line.value)
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor($r('app.color.fg1'))
            .fontFeature('"tnum" on')
            .maxLines(1)
        }
        .padding({ left: Metrics.s2, right: Metrics.s2, top: WebMetrics.s0_5, bottom: WebMetrics.s0_5 })
        .width('100%')
      }, (line: StatLine) => line.label)
    }
    .padding({ left: Metrics.s2, right: Metrics.s2, top: Metrics.s3, bottom: Metrics.s3 })
    .width(WebMetrics.popWide)
  }

  build() {
    Row({ space: WebMetrics.s0_5 }) {
      if (this.retry && this.actions) {
        ActionGlyph({ icon: 'rotateCcw', label: t('common.retry'), onTap: () => this.onRetry() })
      }
      if (replyMarkdown(this.turn).length > 0) {
        ActionGlyph({
          icon: this.copied ? 'check' : 'copy', label: this.copied ? t('code.copied') : t('turn.copy'),
          onTap: () => {
            copyText(this.getUIContext(), replyMarkdown(this.turn));
            this.copied = true;
            setTimeout(() => {
              this.copied = false;
            }, WebMetrics.copiedMs);
          }
        })
      }
      if (shareBlocks(this.turn).length > 0) {
        ActionGlyph({ icon: 'image', label: t('turn.share'), onTap: () => this.share() })
      }
      if (this.actions) {
        ActionGlyph({ icon: 'gitBranch', label: t('turn.fork'), onTap: () => this.onFork() })
      }
      if (statTiles(this.turn).length > 0 || statTokens(this.turn).length > 0) {
        ActionGlyph({
          icon: 'chartNoAxesColumn', label: t('turn.stats'), open: this.stats, onTap: () => {
            this.stats = !this.stats;
          }
        })
          .bindPopup(this.stats, {
            builder: this.statsCard(), placement: Placement.Top, enableArrow: false,
            popupColor: $r('app.color.bg1'), onStateChange: (e) => {
              if (!e.isVisible) {
                this.stats = false;
              }
            }
          })
      }
    }
    .justifyContent(FlexAlign.End)
    // Hugs the reply: gives a gap back to the message's spacing
    .margin({ top: -Metrics.gap })
    .width('100%')
    .onAreaChange((_: Area, area: Area) => {
      this.rowWidth = Number(area.width);
    })
    .bindContentCover(this.shot !== undefined, this.sharePreview(), {
      modalTransition: ModalTransition.ALPHA,
      backgroundColor: Color.Black,
      onDisappear: () => {
        this.shot = undefined;
      }
    })
  }
}

/** Reply-less turns still offer their statistics: a turn has an action row when it settled with something in it */
export function showsTurnActions(turn: TurnX, running: boolean): boolean {
  if (running) {
    return false;
  }
  return replyMarkdown(turn).length > 0 || clockStart(turn) !== undefined;
}
