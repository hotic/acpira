import { AgentBlock, Attachment, QuestionAnswer, QueuedPrompt, SessionControls, SlashCommand, Turn } from '../wire/Wire';
import { Cmp, Metrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { canEditPrompts, dockPlan, goalHeadActions, openQuestion, pendingPlanReview, shortStop } from '../logic/Composing';
import { ModelChoices } from '../logic/ModelCatalog';
import { MentionPersona, SlashCommands } from '../logic/SlashCommands';
import { Lucide } from '../components/Glyphs';
import { AgentMessage, UserBubble } from '../components/TurnViews';
import { Composer } from '../components/Composer';
import { AlertCard, EditingBar, PlanDock, QuestionCard, QueueBubble } from '../components/Dock';
import { attachBeans, Bean, modelBeans } from '../components/composer/Beans';
import { GooLayer } from '../components/composer/GooLayer';
import { ChatBar, ChatBarActions } from '../components/ChatBar';
import { asView, SubagentSummary } from '../wire/Blocks';

/** The view's controls as the plan card reads them */
interface ViewControls {
  controls?: Object;
}

// The chat (apps/ios Chat/ChatView.swift): the bar with back and the title, the thread in the reading column, the
// dock under it (Chat/Dock.swift: question, alert, the prompt being edited, the composer), the to-do card floating over
// the thread's foot, and the liquid picker's beans over everything while one is out. The thread follows new output
// while it is at its end.

@ComponentV2
export struct ChatPage {
  @Require @Param model: AppModel;
  /** False at the detail column's root beside an open sidebar: nothing to go back to */
  @Param showBack: boolean = true;
  /** Screenshots (`-popup` on iOS): focus, context, quotes, edit, expand */
  @Param popup: string = '';
  @Event onBack: () => void = () => {};
  /** Where the bar's entries lead (list agent) */
  @Param actions: ChatBarActions = new ChatBarActions();
  /** The alert card's ✕ hides it for this many turns (the transcript row stays) */
  @Local dismissedAlert: number = -1;
  private scroller: Scroller = new Scroller();
  private atEnd: boolean = true;

  @Monitor('model.view')
  onView(): void {
    // A new or longer thread keeps its end in view, unless the reader scrolled up
    if (this.atEnd) {
      setTimeout(() => {
        this.scroller.scrollEdge(Edge.Bottom);
      }, 0);
    }
  }

  /** Rows of turns a patch kept are the same objects: their keys stay, so only the changed tail draws again */
  turnKey(turn: Turn, i: number): string {
    return i < this.model.stableTurns ? `${i}` : `${i}@${this.model.view?.rev ?? 0}:${turn.blocks?.length ?? 0}`;
  }

  /** The composer's pickers act on a rewritten prompt's own selections while one is edited */
  private controls(): SessionControls | undefined {
    const view = this.model.view;
    if (view === undefined) {
      return undefined;
    }
    return this.model.draftFor(view.id).edit?.controls ?? view.controls;
  }

  /** The open chip's beans, for the screen's layer; a closing picker keeps the beans it goes home with */
  private beans(): Bean[] {
    const goo = this.model.goo;
    if (goo.last === 'attach') {
      return attachBeans(goo, false, this.model.personas, (p: MentionPersona) => this.summon(p));
    }
    const model = this.model;
    const choices = new ModelChoices(this.controls(), model.hiddenModels,
      (id: string, value: string) => model.setConfig(id, value));
    return modelBeans(choices, goo, {
      setMode: (mode: string) => model.setMode(mode),
      setConfig: (id: string, value: string) => model.setConfig(id, value)
    });
  }

  /** SummonMenu: `@name ` at the draft's end, the field focused */
  private summon(persona: MentionPersona): void {
    const view = this.model.view;
    if (view === undefined) {
      return;
    }
    const draft = this.model.draftFor(view.id);
    draft.text = SlashCommands.summon(draft.text, persona.name, draft.text.length, 0).text;
    draft.focusKey++;
  }

  /** While a plan waits, a send revises it */
  private placeholder(): string {
    const view = this.model.view;
    if (view === undefined || this.model.draftFor(view.id).edit !== undefined) {
      return '';
    }
    const review = pendingPlanReview(view.turns);
    const revise = review?.options?.some((o) => o.kind === 'reject_once' && !o.id.toLowerCase().includes('exit')) ?? false;
    return revise ? t('composer.placeholder.revisePlan') : '';
  }

  private question(): AgentBlock | undefined {
    return this.model.view !== undefined ? openQuestion(this.model.view) : undefined;
  }

  private alertTurn(): Turn | undefined {
    return this.model.view !== undefined ? shortStop(this.model.view) : undefined;
  }

  private receded(): boolean {
    return this.model.goo.open !== undefined;
  }

  // Transcript (transcript agent): what the turns read from the view beyond their own turn

  /** The agent's advertised command names: the prompt bubbles tint them */
  commandNames(): string[] {
    const view = this.model.view;
    return view === undefined ? [] : (view.commands ?? []).map((c: SlashCommand) => c.name);
  }

  subagents(): SubagentSummary[] {
    const view = this.model.view;
    return view === undefined ? [] : (asView(view).subagents ?? []);
  }

  /** The session's controls (the plan card's execution model), read loosely */
  planControls(): Object | undefined {
    const view = this.model.view;
    return view === undefined ? undefined : (view as Object as ViewControls).controls;
  }

  build() {
    Column() {
      // The bar and its "…" menu (components/ChatBar.ets)
      ChatBar({ model: this.model, actions: this.actions })
        .blur(this.receded() ? Cmp.gooBlur : 0)

      if (this.model.view === undefined) {
        LoadingProgress()
          .color($r('app.color.fg3'))
          .width(Metrics.barCtl)
          .height(Metrics.barCtl)
          .layoutWeight(1)
      } else {
        Stack({ alignContent: Alignment.Bottom }) {
          this.thread()
          // The to-do card floats over the thread's foot, headed by the session goal
          PlanDock({
            entries: dockPlan(this.model.view.turns, this.model.view.running), running: this.model.view.running,
            goal: this.model.view.goal,
            goalActions: this.model.view.goal !== undefined ? goalHeadActions(this.model.view.goal, this.model.view.goalActions) : [],
            onGoal: (action: string) => this.model.goal(action)
          })
            .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Cmp.dockGap })
            .constraintSize({ maxWidth: Metrics.contentW + Metrics.pad * 2 })
        }
        .layoutWeight(1)
        .blur(this.receded() ? Cmp.gooBlur : 0)

        this.dockCards()

        Row() {
          Composer({
            draft: this.model.draftFor(this.model.view.id),
            placeholder: this.placeholder(),
            running: this.model.view.running,
            controls: this.controls(),
            hidden: this.model.hiddenModels,
            usage: this.model.view.usage,
            compactAt: this.model.compactAt,
            commands: this.model.view.commands ?? [],
            personas: this.model.personas,
            files: this.model.files,
            goo: this.model.goo,
            popup: this.popup,
            onSend: (text: string, staged: Attachment[]) => this.model.composerSend(text, staged),
            onStop: () => this.model.stop(),
            onCompact: () => this.model.compact()
          })
        }
        .justifyContent(FlexAlign.Center)
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Cmp.dockGap, bottom: Metrics.s2 })
        .width('100%')
        .zIndex(2)
      }

      // The picker's backdrop, under the composer and over the rest of the page
      Column()
        .position({ x: 0, y: 0 })
        .width('100%')
        .height('100%')
        .backgroundColor($r('app.color.goo_dim'))
        .opacity(this.receded() ? 1 : 0)
        .animation({ curve: Curve.Friction, duration: Cmp.quickMs * 2 })
        .hitTestBehavior(HitTestMode.None)
        .zIndex(1)
      GooLayer({ goo: this.model.goo, beans: this.beans() })
        .position({ x: 0, y: 0 })
        .width('100%')
        .height('100%')
        .zIndex(3)
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }

  @Builder
  thread() {
    // A chat opens at its end, as the other apps do
    List({ scroller: this.scroller, space: Metrics.msgGap, initialIndex: Math.max(0, (this.model.view?.turns.length ?? 0) - 1) }) {
      ForEach(this.model.view?.turns ?? [], (turn: Turn, i: number) => {
        ListItem() {
          if (turn.role === 'user') {
            UserBubble({ text: turn.text ?? '', turn: turn, commands: this.commandNames() })
              .bindContextMenu(this.promptMenu(i, turn), ResponseType.LongPress)
          } else {
            AgentMessage({
              turn: turn,
              running: (this.model.view?.running ?? false) && i === (this.model.view?.turns.length ?? 0) - 1,
              onPermission: (blockId: string, optionId: string) => this.model.permission(blockId, optionId),
              index: i,
              scope: this.model.view?.id ?? '',
              subagents: this.subagents(),
              controls: this.planControls(),
              agent: this.model.view?.agent ?? '',
              now: (): number => this.model.now(),
              actions: true,
              onFork: (turnIndex: number) => this.model.fork(turnIndex),
              onBuild: (planId: string, optionId: string, configId: string, value: string) =>
                this.model.buildPlan(planId, optionId, configId, value)
            })
              .bindContextMenu(this.replyMenu(i), ResponseType.LongPress)
          }
        }
        .constraintSize({ maxWidth: Metrics.contentW })
        .width('100%')
      }, (turn: Turn, i: number) => this.turnKey(turn, i))
      // Queued prompts are faint bubbles at the end of the thread, in send order
      if ((this.model.view?.queued ?? []).length > 0) {
        ListItem() {
          Column({ space: Metrics.s2 }) {
            ForEach(this.model.view?.queued ?? [], (item: QueuedPrompt) => {
              QueueBubble({
                item: item, steer: this.model.steering(),
                onSend: () => this.model.sendQueued(item.id),
                onEdit: () => this.model.editQueued(item.id),
                onRemove: () => this.model.removeQueued(item.id)
              })
            }, (item: QueuedPrompt) => `${item.id}:${item.sending === true}`)
          }
        }
        .constraintSize({ maxWidth: Metrics.contentW })
        .width('100%')
      }
    }
    .id('chat.thread')
    .alignListItem(ListItemAlign.Center)
    .padding({ left: Metrics.pad + Metrics.s1, right: Metrics.pad + Metrics.s1, top: Metrics.gap, bottom: Metrics.gap })
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.Spring)
    .onReachEnd(() => {
      this.atEnd = true;
    })
    .onScrollIndex((start: number, end: number) => {
      this.atEnd = end >= (this.model.view?.turns.length ?? 0) - 1;
    })
    .height('100%')
    .width('100%')
  }

  /** The surfaces under the thread: the open question, a turn that stopped short, the prompt being rewritten */
  @Builder
  dockCards() {
    Column({ space: Cmp.dockGap }) {
      if (this.question() !== undefined) {
        QuestionCard({
          block: this.question() as AgentBlock,
          onAnswer: (answers: Record<string, QuestionAnswer>, skipped: boolean) =>
            this.model.answer(this.question()?.id ?? '', answers, skipped)
        })
          .id(`question:${this.question()?.id ?? ''}`)
      }
      if (this.model.view !== undefined && this.alertTurn() !== undefined &&
        this.dismissedAlert !== this.model.view.turns.length) {
        AlertCard({
          turn: this.alertTurn() as Turn,
          canCompact: (this.model.view.commands ?? []).some((c: SlashCommand) => c.name === 'compact'),
          onDismiss: () => {
            this.dismissedAlert = this.model.view?.turns.length ?? -1;
          },
          onRetry: () => this.model.retry(),
          onContinue: () => this.model.carryOn(),
          onCompact: () => this.model.compact(),
          onNewSession: () => this.model.continueNew()
        })
      }
      if (this.model.view !== undefined && this.model.draftFor(this.model.view.id).edit !== undefined) {
        EditingBar({
          original: this.model.draftFor(this.model.view.id).edit?.original.text ?? '',
          onCancel: () => this.model.cancelEdit()
        })
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .constraintSize({ maxWidth: Metrics.contentW + Metrics.pad * 2 })
    .width('100%')
    .blur(this.receded() ? Cmp.gooBlur : 0)
  }

  /** A sent prompt's long press: rewrite it in the composer (while the session is ready and idle) */
  @Builder
  promptMenu(index: number, turn: Turn) {
    Menu() {
      if (this.model.view !== undefined && canEditPrompts(this.model.view) && turn.auto !== true) {
        MenuItem({ content: t('history.edit') })
          .onClick(() => this.model.beginEdit(index))
      }
      MenuItem({ content: t('turn.fork') })
        .onClick(() => this.model.fork(index))
    }
  }

  /** A reply's long press: fork the conversation from it */
  @Builder
  replyMenu(index: number) {
    Menu() {
      MenuItem({ content: t('turn.fork') })
        .onClick(() => this.model.fork(index))
    }
  }
}
