import { pasteboard } from '@kit.BasicServicesKit';
import { LengthMetrics } from '@kit.ArkUI';
import { AgentBlock, Attachment, PlanEntry, Question, QuestionAnswer, QuestionOption, QueuedPrompt, SessionGoal, Turn } from '../wire/Wire';
import { Cmp, Metrics, Type } from '../design/Tokens';
import { firstLine, goalHeadMeta, goalStatusLabel, isContextLengthError } from '../logic/Composing';
import { t } from '../store/L10n';
import { Lucide } from './Glyphs';
import { CardButton, CardCorner, DOCK_CARD } from './composer/ComposerBits';

// chat/Shell.tsx's DockStack at phone width (apps/ios Chat/Dock.swift, Chat/QuestionCard.swift, Chat/AlertCard.swift,
// Chat/PromptEdit.swift; apps/android ui/Dock.kt, ui/Cards.kt): the surfaces under the thread share one --dock-gap
// rhythm (question, alert, the prompt being edited, then the composer), the live to-do card headed by the session goal
// floats just above them, and queued prompts are faint bubbles at the thread's end.

const KEYS: string = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';

/** One question's picks: chosen option ids, the Other field's state and its text */
interface QPick {
  options: string[];
  other: boolean;
  text: string;
}

function emptyPick(): QPick {
  return { options: [], other: false, text: '' };
}

/** What the card sends: answered questions only, a string or a list of option ids (and Other's text) each */
function collected(qs: Question[], picks: Map<string, QPick>): Record<string, QuestionAnswer> {
  const out: Record<string, QuestionAnswer> = {};
  for (const q of qs) {
    const p = picks.get(q.id) ?? emptyPick();
    const text = p.text.trim();
    if (q.kind === 'text') {
      if (text.length > 0) {
        out[q.id] = text;
      }
    } else if (q.kind === 'multiple') {
      const list = p.options.concat(p.other && text.length > 0 ? [text] : []);
      if (list.length > 0) {
        out[q.id] = list;
      }
    } else if (p.other) {
      if (text.length > 0) {
        out[q.id] = text;
      }
    } else if (p.options.length > 0) {
      out[q.id] = p.options[0];
    }
  }
  return out;
}

/**
 * The open question docked above the composer (QuestionCard.swift, chat/Questions.tsx): one question per page,
 * lettered options, an Other field, Skip on the left and Next / Submit on the right of the footer
 */
@ComponentV2
export struct QuestionCard {
  @Require @Param block: AgentBlock;
  @Event onAnswer: (answers: Record<string, QuestionAnswer>, skipped: boolean) => void = () => {};
  @Local page: number = 0;
  @Local picks: Map<string, QPick> = new Map<string, QPick>();

  private questions(): Question[] {
    return this.block.questions ?? [];
  }

  private current(): Question {
    const qs = this.questions();
    return qs[Math.max(0, Math.min(this.page, qs.length - 1))];
  }

  private pickOf(q: Question): QPick {
    return this.picks.get(q.id) ?? emptyPick();
  }

  private setPick(q: Question, p: QPick): void {
    const next = new Map<string, QPick>(this.picks);
    next.set(q.id, p);
    this.picks = next;
  }

  private canAdvance(): boolean {
    const qs = this.questions();
    const answers = collected(qs, this.picks);
    const last = this.page >= qs.length - 1;
    if (last) {
      return Object.keys(answers).length > 0 && qs.every((q: Question) => q.required !== true || answers[q.id] !== undefined);
    }
    const q = this.current();
    return q.required !== true || answers[q.id] !== undefined;
  }

  private choose(q: Question, o: QuestionOption): void {
    const p = this.pickOf(q);
    const selected = p.options.includes(o.id);
    if (q.kind === 'multiple') {
      this.setPick(q, {
        options: selected ? p.options.filter((id: string) => id !== o.id) : p.options.concat([o.id]), other: p.other,
        text: p.text
      });
    } else {
      this.setPick(q, { options: [o.id], other: false, text: p.text });
    }
  }

  build() {
    if (this.questions().length > 0) {
      Column() {
        Scroll() {
          Column({ space: Cmp.questionSectionGap }) {
            Row({ space: Metrics.s2 }) {
              Lucide({ name: 'messageCircleQuestion', tint: $r('app.color.fg3'), extent: Cmp.webIcon })
                .margin({ top: (Cmp.text2Line - Cmp.webIcon) / 2 })
              Text(this.current().text)
                .fontSize(Type.text2)
                .lineHeight(Cmp.text2Line)
                .fontWeight(FontWeight.Medium)
                .fontColor($r('app.color.fg_strong'))
                .layoutWeight(1)
              Text(t('question.of', new Map<string, string>([['n', `${this.page + 1}`], ['total', `${this.questions().length}`]])))
                .fontSize(Type.text3)
                .lineHeight(Cmp.text2Line)
                .fontColor($r('app.color.fg3'))
            }
            .alignItems(VerticalAlign.Top)
            .width('100%')
            if (this.page === 0 && this.block.message !== undefined) {
              Text(this.block.message)
                .fontSize(Type.text2)
                .lineHeight(Cmp.text2Line)
                .fontColor($r('app.color.pop_fg2'))
            }
            this.options(this.current())
          }
          .alignItems(HorizontalAlign.Start)
          .padding({ left: Metrics.pad, right: Metrics.pad, top: Cmp.questionTopPad, bottom: Cmp.questionSectionGap })
        }
        .scrollBar(BarState.Off)
        .edgeEffect(EdgeEffect.None)
        .constraintSize({ maxHeight: Cmp.questionBody })
        Row({ space: Metrics.gap }) {
          CardButton({ label: t('question.skip'), onTap: () => this.onAnswer(collected(this.questions(), this.picks), true) })
            .id('question.skip')
          Blank()
          if (this.page > 0) {
            CardButton({ label: t('question.prev'), onTap: () => {
              this.page = this.page - 1;
            } })
          }
          CardButton({
            label: t(this.page >= this.questions().length - 1 ? 'question.submit' : 'question.next'), primary: true,
            enable: this.canAdvance(), onTap: () => {
              if (this.page >= this.questions().length - 1) {
                this.onAnswer(collected(this.questions(), this.picks), false);
              } else {
                this.page = this.page + 1;
              }
            }
          })
        }
        .width('100%')
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s1, bottom: Metrics.pad })
      }
      .attributeModifier(DOCK_CARD)
      .id('question')
    }
  }

  @Builder
  options(q: Question) {
    Column() {
      if (q.kind === 'multiple') {
        Text(t('question.multiple'))
          .fontSize(Type.text3)
          .lineHeight(Cmp.text3Line)
          .fontColor($r('app.color.pop_fg2'))
          .margin({ bottom: Cmp.questionSectionGap })
      }
      ForEach(q.options, (o: QuestionOption, i: number) => {
        Row({ space: Metrics.s2 }) {
          this.optionKey(KEYS.charAt(i), this.pickOf(q).options.includes(o.id))
          Column({ space: Cmp.questionDescriptionGap }) {
            Text(o.label)
              .fontSize(Type.text2)
              .lineHeight(Cmp.text2Line)
              .fontColor($r('app.color.fg1'))
            if (o.description !== undefined) {
              Text(o.description)
                .fontSize(Type.text3)
                .lineHeight(Cmp.text3Line)
                .fontColor($r('app.color.pop_fg2'))
            }
          }
          .alignItems(HorizontalAlign.Start)
          .layoutWeight(1)
        }
        .optionRow(this.pickOf(q).options.includes(o.id))
        .onClick(() => this.choose(q, o))
      }, (o: QuestionOption) => o.id)
      if (q.kind !== 'text' && q.other === true) {
        Row({ space: Metrics.s2 }) {
          this.optionKey(KEYS.charAt(q.options.length), this.pickOf(q).other)
          TextInput({ text: this.pickOf(q).text, placeholder: t('question.other') })
            .answerField()
            .onChange((value: string) => {
              const p = this.pickOf(q);
              this.setPick(q, { options: q.kind === 'single' ? [] : p.options, other: true, text: value });
            })
        }
        .optionRow(this.pickOf(q).other)
      }
      if (q.kind === 'text') {
        TextInput({
          text: this.pickOf(q).text,
          placeholder: t(q.numeric === true ? 'question.numberPlaceholder' : 'question.textPlaceholder')
        })
          .answerField()
          .type(q.numeric === true ? InputType.NUMBER_DECIMAL : InputType.Normal)
          .height(Metrics.ctl)
          .padding({ left: Metrics.gap, right: Metrics.gap })
          .border({ width: Metrics.hairline, color: $r('app.color.line_strong') })
          .borderRadius(Metrics.rMd)
          .onChange((value: string) => this.setPick(q, { options: [], other: false, text: value }))
      }
    }
    .width('100%')
  }

  /** The option's key ring, a check once picked */
  @Builder
  optionKey(letter: string, selected: boolean) {
    Stack() {
      if (selected) {
        Lucide({ name: 'check', tint: $r('app.color.fg1'), extent: Cmp.webChevron })
      } else {
        Text(letter)
          .fontSize(Cmp.optionKey)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.pop_fg2'))
      }
    }
    .width(Metrics.icon)
    .height(Metrics.icon)
    .borderRadius(Metrics.icon / 2)
    .border({ width: Metrics.hairline, color: $r('app.color.line_strong') })
    .margin({ top: (Cmp.text2Line - Metrics.icon) / 2 })
  }
}

@Extend(Row)
function optionRow(selected: boolean) {
  .alignItems(VerticalAlign.Top)
  .width('100%')
  .padding({ top: Cmp.questionOptionPad, bottom: Cmp.questionOptionPad, left: Cmp.questionOptionPad, right: Cmp.questionOptionPad })
  .margin({ left: -Cmp.questionOptionPad, right: -Cmp.questionOptionPad })
  .borderRadius(Metrics.rMd)
  .backgroundColor(selected ? $r('app.color.active') : Color.Transparent)
}

@Extend(TextInput)
function answerField() {
  .fontSize(Type.text2)
  .fontColor($r('app.color.fg1'))
  .placeholderColor($r('app.color.pop_fg2'))
  .placeholderFont({ size: Type.text2 })
  .backgroundColor(Color.Transparent)
  .borderRadius(0)
  .padding(0)
  .layoutWeight(1)
}

/**
 * A turn that stopped short (AlertCard.swift, Alert.tsx): what happened and ✕ on the first line, the agent's words under
 * it, then the copyable detail at the start and the way on at the end: retry for an error (plus "continue in a new
 * session" when the failure names it), carry on for a limit, nothing for a refusal; a context grown too long offers
 * compaction instead
 */
@ComponentV2
export struct AlertCard {
  @Require @Param turn: Turn;
  /** Only when the agent advertises `/compact` */
  @Param canCompact: boolean = false;
  @Event onDismiss: () => void = () => {};
  @Event onRetry: () => void = () => {};
  @Event onContinue: () => void = () => {};
  @Event onCompact: () => void = () => {};
  @Event onNewSession: () => void = () => {};
  @Local copied: boolean = false;

  private stop(): string {
    return this.turn.stop ?? 'error';
  }

  private contextTooLong(): boolean {
    return this.stop() === 'error' && isContextLengthError(this.turn.error);
  }

  private detail(): string {
    const err = this.turn.error;
    const parts: string[] = [];
    if (err?.code !== undefined) {
      parts.push(`${err.code}`);
    }
    if (err?.kind !== undefined && err.kind.length > 0) {
      parts.push(err.kind);
    }
    return parts.join(t('common.metaSep'));
  }

  private copyable(): string {
    return [this.turn.error?.message ?? '', this.detail()].filter((s: string) => s.length > 0).join('\n');
  }

  private message(): string | undefined {
    const stop = this.stop();
    if (this.contextTooLong()) {
      return t(this.canCompact ? 'alert.contextLength.text' : 'alert.contextLength.unsupported');
    }
    if (stop === 'error') {
      const m = this.turn.error?.message ?? '';
      return m.length > 0 ? m : t('alert.error.unknown');
    }
    if (stop === 'refusal') {
      return t('alert.refusal.text');
    }
    if (stop === 'max_tokens') {
      return t('alert.maxTokens.text');
    }
    if (stop === 'max_turn_requests') {
      return t('alert.maxTurns.text');
    }
    return undefined;
  }

  private title(): string {
    const stop = this.stop();
    if (this.contextTooLong()) {
      return t('alert.contextLength.title');
    }
    if (stop === 'refusal') {
      return t('alert.refusal.title');
    }
    if (stop === 'max_tokens') {
      return t('alert.maxTokens.title');
    }
    if (stop === 'max_turn_requests') {
      return t('alert.maxTurns.title');
    }
    return t('alert.error.title');
  }

  private retry(): boolean {
    const actions = this.turn.error?.actions;
    return this.stop() === 'error' && !this.contextTooLong() && (actions === undefined || actions.includes('retry'));
  }

  private fresh(): boolean {
    return this.stop() === 'error' && !this.contextTooLong() && (this.turn.error?.actions ?? []).includes('new_session');
  }

  /** How many ways on the action row holds */
  private ways(): number {
    return [this.fresh(), this.retry(), this.carry(), this.contextTooLong() && this.canCompact]
      .filter((on: boolean) => on).length;
  }

  private carry(): boolean {
    return this.stop() === 'max_tokens' || this.stop() === 'max_turn_requests';
  }

  build() {
    Column({ space: Metrics.s2 }) {
      Row({ space: Metrics.s2 }) {
        Lucide({ name: 'triangleAlert', tint: $r('app.color.warn'), extent: Metrics.icon })
        Text(this.title())
          .fontSize(Type.text2)
          .lineHeight(Cmp.text2Line)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg_strong'))
          .layoutWeight(1)
        CardCorner({ icon: 'x', label: t('common.close'), onTap: () => this.onDismiss() })
          .id('alert.close')
      }
      .width('100%')
      if (this.message() !== undefined) {
        Text(this.message())
          .fontSize(Type.text3)
          .lineHeight(Cmp.text3Line)
          .fontColor($r('app.color.pop_fg2'))
      }
      if (this.copyable().length > 0 || this.retry() || this.fresh() || this.carry() ||
        (this.contextTooLong() && this.canCompact)) {
        Flex({ justifyContent: FlexAlign.SpaceBetween, alignItems: ItemAlign.Center, wrap: FlexWrap.Wrap }) {
          if (this.copyable().length > 0) {
            CardButton({
              // The detail rides in the label only while one way on shares the row (CardActions' shorter forms)
              label: this.copied ? t('alert.copied') : this.detail().length > 0 && this.ways() < 2 ?
                t('alert.copyWith', new Map<string, string>([['detail', this.detail()]])) : t('alert.copy'),
              onTap: () => this.copy()
            })
              .flexShrink(1)
              .margin({ right: Metrics.s2 })
          } else {
            Blank()
          }
          Row({ space: Metrics.s2 }) {
            if (this.fresh()) {
              CardButton({ label: t('notice.continueNew'), onTap: () => this.onNewSession() })
            }
            if (this.retry()) {
              CardButton({ label: t('common.retry'), primary: true, onTap: () => this.onRetry() })
                .id('alert.retry')
            }
            if (this.carry()) {
              CardButton({ label: t('alert.continue'), primary: true, onTap: () => this.onContinue() })
                .id('alert.continue')
            }
            if (this.contextTooLong() && this.canCompact) {
              CardButton({ label: t('alert.contextLength.compact'), primary: true, onTap: () => this.onCompact() })
            }
          }
        }
        .width('100%')
        .margin({ top: Metrics.s1 })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Cmp.s3, bottom: Cmp.s3 })
    .attributeModifier(DOCK_CARD)
    .id('alert')
  }

  private copy(): void {
    try {
      pasteboard.getSystemPasteboard().setDataSync(pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, this.copyable()));
    } catch (e) {
      // No pasteboard (the Previewer)
    }
    this.copied = true;
    setTimeout(() => {
      this.copied = false;
    }, Cmp.quickMs * 10);
  }
}

/** The line above the composer while a sent prompt is being rewritten (EditingBar): what it was, and ✕ to give it back */
@ComponentV2
export struct EditingBar {
  @Param original: string = '';
  @Event onCancel: () => void = () => {};

  build() {
    Column() {
      Row({ space: Metrics.s2 }) {
        Lucide({ name: 'pencil', tint: $r('app.color.fg3'), extent: Metrics.icon })
          .margin({ right: Metrics.s1 })
        Text(t('mobile.edit.title'))
          .fontSize(Type.text2)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
          .flexShrink(0)
        Text(firstLine(this.original) ?? '')
          .fontSize(Type.text2)
          .fontColor($r('app.color.pop_fg3'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .layoutWeight(1)
        CardCorner({ icon: 'x', label: t('history.cancel'), onTap: () => this.onCancel() })
          .id('edit.cancel')
      }
      .width('100%')
      .constraintSize({ minHeight: Metrics.row })
      .padding({ left: Metrics.pad, right: Metrics.s1, top: Metrics.s1, bottom: Metrics.s1 })
    }
    .attributeModifier(DOCK_CARD)
    .id('edit.bar')
  }
}

/** A plan entry's status dot (PlanDot): a check once done, the accent ring while in progress, a faint ring otherwise */
@Builder
function PlanDot(status: string) {
  Stack() {
    if (status === 'completed') {
      Lucide({ name: 'check', tint: $r('app.color.bg0'), extent: Metrics.s2 })
    } else if (status === 'in_progress') {
      Column()
        .width(Cmp.s1_5)
        .height(Cmp.s1_5)
        .borderRadius(Cmp.s1_5 / 2)
        .backgroundColor($r('app.color.brand'))
    }
  }
  .width(Cmp.webChevron)
  .height(Cmp.webChevron)
  .borderRadius(Cmp.webChevron / 2)
  .backgroundColor(status === 'completed' ? $r('app.color.fg3') : Color.Transparent)
  .border({
    width: status === 'completed' ? 0 : Cmp.planRing,
    color: status === 'in_progress' ? $r('app.color.brand') : $r('app.color.fg3')
  })
}

/**
 * PlanBar's card (Dock.swift PlanCard): the to-do title with the done count and a caret; folded it shows the current
 * entry only (or "all n done"), unfolded every entry, scrolling past seven rows. A session goal takes the head row, so
 * goal and to-dos share one card
 */
@ComponentV2
export struct PlanDock {
  @Param entries: PlanEntry[] | undefined = undefined;
  @Param running: boolean = false;
  @Param goal: SessionGoal | undefined = undefined;
  @Param goalActions: string[] = [];
  @Event onGoal: (action: string) => void = () => {};
  @Local open: boolean = false;

  private current(): number {
    const e = this.entries ?? [];
    const working = e.findIndex((x: PlanEntry) => x.status === 'in_progress');
    return working >= 0 ? working : e.findIndex((x: PlanEntry) => x.status === 'pending');
  }

  private foldable(): boolean {
    return (this.entries?.length ?? 0) > 1;
  }

  private done(): number {
    return (this.entries ?? []).filter((x: PlanEntry) => x.status === 'completed').length;
  }

  build() {
    if (this.entries !== undefined || this.goal !== undefined) {
      Column({ space: Cmp.planSectionGap }) {
        if (this.goal !== undefined) {
          this.goalHead(this.goal)
        } else if (this.entries !== undefined) {
          Row({ space: Metrics.s2 }) {
            Stack() {
              Lucide({ name: 'listTodo', tint: $r('app.color.fg3'), extent: Cmp.webIcon })
            }
            .width(Cmp.webLead)
            Text(t('plan.todoTitle'))
              .fontSize(Type.text2)
              .fontColor($r('app.color.fg2'))
              .layoutWeight(1)
            this.count()
          }
          .height(Cmp.rowDense)
          .width('100%')
          .onClick(() => {
            if (this.foldable()) {
              this.open = !this.open;
            }
          })
        }
        if (this.entries !== undefined) {
          Scroll() {
            Column() {
              if (this.foldable() && !this.open && this.current() < 0) {
                this.entry({ title: t('plan.completed', new Map<string, string>([['n', `${this.entries.length}`]])), status: 'completed' },
                  true)
              }
              ForEach(this.entries, (e: PlanEntry, i: number) => {
                if (this.open || !this.foldable() || i === this.current()) {
                  this.entry(e, false)
                }
              }, (e: PlanEntry, i: number) => `${i}:${e.status}:${e.title}`)
            }
          }
          .scrollBar(BarState.Off)
          .enableScrollInteraction(this.open)
          .constraintSize({ maxHeight: Cmp.planMax })
          .align(Alignment.Top)
        }
      }
      .alignItems(HorizontalAlign.Start)
      .padding({ left: Metrics.pad, right: Metrics.pad, top: Cmp.planPadY, bottom: Cmp.planPadY })
      .attributeModifier(DOCK_CARD)
      .animation({ duration: Cmp.quickMs * 2, curve: Curve.Friction })
      .id('plan.dock')
    }
  }

  @Builder
  entry(e: PlanEntry, summary: boolean) {
    Row({ space: Metrics.s2 }) {
      Stack() {
        PlanDot(e.status)
      }
      .width(Cmp.webLead)
      .height(Cmp.text2Line)
      Text(e.title)
        .fontSize(Type.text2)
        .lineHeight(Cmp.text2Line)
        .fontColor(summary || e.status === 'completed' ? $r('app.color.fg2') :
          e.status === 'in_progress' ? $r('app.color.fg_strong') : $r('app.color.fg1'))
        .layoutWeight(1)
    }
    .alignItems(VerticalAlign.Top)
    .width('100%')
    .padding({ top: Cmp.planEntryPadY, bottom: Cmp.planEntryPadY })
  }

  /** The done count and the caret that folds the list */
  @Builder
  count() {
    if (this.entries !== undefined) {
      Row({ space: Metrics.s2 }) {
        Text(`${this.done()}/${this.entries.length}`)
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .fontFeature('"tnum" on')
        if (this.foldable()) {
          Lucide({ name: 'chevronDown', tint: $r('app.color.fg3'), extent: Metrics.icon })
            .rotate({ angle: this.open ? Cmp.halfTurn : 0 })
        }
      }
    }
  }

  /** GoalHead: the glyph, the status and the objective on one line, the controls' menu, then the to-do count */
  @Builder
  goalHead(goal: SessionGoal) {
    Row({ space: Metrics.s2 }) {
      Stack() {
        if (goal.status === 'complete') {
          Lucide({ name: 'circleCheck', tint: $r('app.color.ok'), extent: Cmp.webIcon })
        } else if (goal.status === 'blocked' || goal.status === 'limited') {
          Lucide({ name: 'triangleAlert', tint: $r('app.color.warn'), extent: Cmp.webIcon })
        } else {
          Lucide({
            name: 'goal', tint: goal.status === 'active' ? $r('app.color.brand') : $r('app.color.fg3'), extent: Cmp.webIcon
          })
        }
      }
      .width(Cmp.webLead)
      Text() {
        Span(goalStatusLabel(goal.status))
          .fontColor($r('app.color.fg1'))
        Span(`  ${firstLine(goal.objective) ?? ''}`)
          .fontColor(goal.status === 'paused' || goal.status === 'complete' ? $r('app.color.fg3') : $r('app.color.fg2'))
      }
      .fontSize(Type.text2)
      .maxLines(1)
      .textOverflow({ overflow: TextOverflow.Ellipsis })
      .layoutWeight(1)
      if (this.goalActions.length === 0 && goalHeadMeta(goal) !== undefined) {
        Text(goalHeadMeta(goal))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
      }
      if (this.goalActions.length > 0) {
        Stack() {
          Lucide({ name: 'ellipsis', tint: $r('app.color.fg2'), extent: Cmp.webIcon })
        }
        .id('goal.menu')
        .width(Cmp.webCtlSm)
        .height(Cmp.webCtlSm)
        .bindMenu(this.goalMenu(goal))
      }
      this.count()
    }
    .height(Cmp.rowDense)
    .width('100%')
    .onClick(() => {
      if (this.foldable()) {
        this.open = !this.open;
      }
    })
  }

  /** The controls the agent advertised, under the full objective and the spend */
  @Builder
  goalMenu(goal: SessionGoal) {
    Menu() {
      MenuItem({ content: goal.objective + (goalHeadMeta(goal) !== undefined ? `\n${goalHeadMeta(goal)}` : '') })
        .enabled(false)
      ForEach(this.goalActions, (action: string) => {
        MenuItem({
          content: t(action === 'pause' ? 'goal.pause' : action === 'resume' ? 'goal.resume' : 'goal.clear')
        })
          .contentFontColor(action === 'clear' ? $r('app.color.danger') : $r('app.color.fg1'))
          .onClick(() => this.onGoal(action))
      }, (action: string) => action)
    }
  }
}

/**
 * A queued prompt (QueueBubble): the sent prompt's bubble, right-aligned after the running turn and faint until it goes;
 * a tap opens a menu: send now (or steer, when the agent takes it mid-turn), edit and remove, under when it goes
 */
@ComponentV2
export struct QueueBubble {
  @Require @Param item: QueuedPrompt;
  @Param steer: boolean = false;
  @Event onSend: () => void = () => {};
  @Event onEdit: () => void = () => {};
  @Event onRemove: () => void = () => {};

  build() {
    Column({ space: Metrics.s2 }) {
      if ((this.item.attachments ?? []).length > 0) {
        Flex({ justifyContent: FlexAlign.End, wrap: FlexWrap.Wrap, space: { main: LengthMetrics.vp(Metrics.s1), cross: LengthMetrics.vp(Metrics.s1) } }) {
          ForEach(this.item.attachments ?? [], (a: Attachment) => {
            Row({ space: Metrics.s1 }) {
              Lucide({ name: a.kind === 'image' ? 'image' : 'fileText', tint: $r('app.color.fg3'), extent: Cmp.webIcon })
              Text(a.kind === 'quote' ? t('quote.chipOne') : a.name ?? '')
                .fontSize(Type.text3)
                .fontWeight(FontWeight.Medium)
                .fontColor($r('app.color.fg2'))
                .maxLines(1)
            }
            .height(Cmp.webCtlSm)
            .padding({ left: Metrics.s2, right: Metrics.s2 })
            .borderRadius(Metrics.rSm)
            .backgroundColor($r('app.color.chip'))
          }, (a: Attachment, i: number) => `${i}`)
        }
      }
      if (this.item.text.trim().length > 0) {
        Text(this.item.text.trim())
          .fontSize(Type.body)
          .lineHeight(Type.bodyLine)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(Cmp.queuedLines)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .padding({ left: Cmp.queuedPadX, right: Cmp.queuedPadX, top: Cmp.s2_5, bottom: Cmp.s2_5 })
          .backgroundColor($r('app.color.bubble_bg'))
          .borderRadius(Metrics.rBubble)
      }
    }
    .alignItems(HorizontalAlign.End)
    .opacity(Cmp.queuedBubble)
    .padding({ left: Metrics.bubbleInset })
    .width('100%')
    .id('queue.bubble')
    .enabled(this.item.sending !== true)
    .bindMenu(this.menu())
  }

  @Builder
  menu() {
    Menu() {
      MenuItem({ content: t(this.item.sending === true ? 'queue.sending' : 'queue.title') })
        .enabled(false)
      MenuItem({ content: t(this.steer ? 'queue.steer' : 'queue.sendNow') })
        .onClick(() => this.onSend())
      MenuItem({ content: t('queue.edit') })
        .onClick(() => this.onEdit())
      MenuItem({ content: t('queue.remove') })
        .contentFontColor($r('app.color.danger'))
        .onClick(() => this.onRemove())
    }
  }
}
