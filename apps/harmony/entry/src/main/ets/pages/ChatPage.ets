import { LengthMetrics } from '@kit.ArkUI';
import {
  AgentBlock, Attachment, earlierTurns, QuestionAnswer, QueuedPrompt, SessionControls, SessionView, SlashCommand, Turn
} from '../wire/Wire';
import { Cmp, ListMetrics, Metrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { canEditPrompts, dockPlan, goalHeadActions, openQuestion, pendingPlanReview, shortStop } from '../logic/Composing';
import { ModelChoices } from '../logic/ModelChoices';
import { MentionPersona, SlashCommands } from '../logic/SlashCommands';
import { Lucide } from '../components/Glyphs';
import { AgentMessage, UserBubble } from '../components/TurnViews';
import { Composer } from '../components/Composer';
import { AlertCard, EditingBar, PlanDock, QuestionCard, QueueBubble } from '../components/Dock';
import { attachBeans, Bean, modelBeans } from '../components/composer/Beans';
import { GooLayer } from '../components/composer/GooLayer';
import { ChatBar, ChatBarActions } from '../components/ChatBar';
import { CardButton } from '../components/Cards';
import { topFade } from '../components/Chrome';
import { turnKey } from '../store/Thread';

// The chat (apps/ios Chat/ChatView.swift): the bar with back and the title, the thread in the reading column, the
// dock under it (Chat/Dock.swift: question, alert, the prompt being edited, the composer), the to-do card floating over
// the thread's foot, and the liquid picker's beans over everything while one is out. The thread follows new output
// while it is at its end. A view that starts at the session's tail (SessionView.earlier) gets the part ahead as the
// reader scrolls near the thread's top, under a spinner row, and what is in sight keeps its place when it lands. Only
// the reader's own scrolling asks: a finished turn's steps sit in a closed fold, so the parts of a huge cut turn add no
// height, and a thread that asked again whenever its top stayed in sight would pull the whole turn part after part.

/** Where the thread's rows sit for a view: its session, where its window starts, its turns, and the spinner row ahead */
interface Shown {
  id: string;
  ahead: number;
  cut: number;
  turns: number;
  head: number;
}

function shownOf(view: SessionView | undefined): Shown {
  const shown: Shown = {
    id: view?.id ?? '', ahead: earlierTurns(view), cut: view?.earlier?.blocks ?? 0, turns: view?.turns.length ?? 0,
    head: view?.earlier !== undefined ? 1 : 0
  };
  return shown;
}

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
  /** The alert card's ✕ hides it for this stop (alertKey; the transcript row stays) */
  @Local dismissedAlert: string = '';
  /** The to-do card's height over the thread's foot, so the last turn can scroll clear of it */
  @Local planHeight: number = 0;
  /** The thread has moved off its top: the fade under the bar shows */
  @Local scrolled: boolean = false;
  private scroller: Scroller = new Scroller();
  private atEnd: boolean = true;
  /** The thread's first row in sight (a list index) and its height: what holds the reader's place when a part lands */
  private first: number = 0;
  private threadH: number = 0;
  /** The view the thread's rows were last built for, and the model's count of whole views when it came */
  private shown: Shown = shownOf(undefined);
  private windows: number = 0;
  /** Parts the thread may still ask for on its own: a fresh window too short to scroll gives the reader no way to */
  private fill: number = 0;
  /** The gesture under way began on a thread too short to scroll (only a pull down at its top asks), and has asked */
  private shortPull: boolean = false;
  private pulled: boolean = false;

  aboutToAppear(): void {
    this.model.thread.follow(this.model.view);
    this.shown = shownOf(this.model.view);
    this.windows = this.model.windows;
    this.fill = Metrics.earlierFill;
  }

  @Monitor('model.view')
  onView(): void {
    const view = this.model.view;
    this.model.thread.follow(view);
    // A monitor runs before the rows update: the list still lays `held` out
    const held = this.shown;
    this.shown = shownOf(view);
    const moved = view !== undefined && held.id === view.id && (held.ahead !== this.shown.ahead || held.cut !== this.shown.cut);
    // A whole view from the engine, not a patch or a part in front: a fresh window may be filled again
    const fresh = this.windows !== this.model.windows;
    if (fresh) {
      this.windows = this.model.windows;
      this.fill = Metrics.earlierFill;
    }
    // A new or longer thread keeps its end in view, unless the reader scrolled up
    if (this.atEnd) {
      if (moved) {
        // Rows came in front: asked for now, the end is kept by the layout that brings them
        this.scroller.scrollEdge(Edge.Bottom);
      }
      setTimeout(() => {
        this.scroller.scrollEdge(Edge.Bottom);
      }, 0);
    } else if (moved) {
      this.holdPlace(held);
    }
    if ((fresh || moved) && this.fill > 0 && this.shown.head > 0) {
      // Once the list has laid the new rows out (a list already on the page reports no layout of its own)
      setTimeout(() => this.fillShort(), Metrics.earlierSettleMs);
    }
  }

  /**
   * The view's window moved (earlier turns came in front, the first turn got its leading blocks, or the engine sent the
   * session's tail again) while the reader is away from the end: the first turn in sight keeps its place. List indices
   * shift with the turns in front and ForEach keeps no position, so the list jumps to the same turn's new index in the
   * layout that brings the new rows. The row keeps its bottom edge, since a cut turn grows at its head
   */
  private holdPlace(held: Shown): void {
    const row = Math.max(this.first, held.head);
    const turn = row - held.head;
    // The same turn in the new view, by its index in the session
    const next = turn + held.ahead - this.shown.ahead;
    let rect: RectResult;
    try {
      rect = this.scroller.getItemRect(row);
    } catch (e) {
      // No list on the page to hold a place in
      return;
    }
    // A row out of sight has no rect: nothing to hold, the list keeps what it shows
    if (turn >= held.turns || next < 0 || next >= this.shown.turns || rect.height <= 0 || this.threadH <= 0) {
      return;
    }
    // END puts the row's bottom on the content's end; a positive extra offset scrolls on towards the end from there
    const foot = this.threadH - this.footPad();
    this.scroller.scrollToIndex(next + this.shown.head, false, ScrollAlign.END,
      { extraOffset: LengthMetrics.vp(foot - (rect.y + rect.height)) });
  }

  /**
   * Asks for the part ahead of a windowed view when the reader's own scrolling leaves the thread's top within reach.
   * A drag reports Scroll and its inertia Fling; a part landing, the jump that holds the place and any other layout
   * report Idle and never ask, so after a landing the next part waits for the reader to scroll on
   */
  private readerScrolled(state: ScrollState): void {
    if ((state !== ScrollState.Scroll && state !== ScrollState.Fling) || this.model.view?.earlier === undefined) {
      return;
    }
    const offset = this.scroller.currentOffset()?.yOffset;
    if (offset === undefined || offset > Metrics.earlierAhead) {
      return;
    }
    if (this.shortPull) {
      // Nothing to scroll: a pull down past the top asks, once however long it is held
      if (this.pulled || offset >= 0) {
        return;
      }
      this.pulled = true;
    }
    this.model.loadEarlier();
  }

  /** The thread's content does not fill the page: there is nothing to scroll */
  private short(): boolean {
    // No offset while the list is off the page
    const offset = this.scroller.currentOffset()?.yOffset;
    return offset !== undefined && offset <= 0 && this.scroller.isAtEnd();
  }

  /**
   * A fresh window too short to scroll asks for the part ahead on its own, since the reader cannot: a few parts at
   * most (they may add no height), and no more once the thread scrolls
   */
  private fillShort(): void {
    if (this.fill <= 0 || this.model.view?.earlier === undefined || this.model.loadingEarlier) {
      return;
    }
    if (!this.short()) {
      this.fill = 0;
      return;
    }
    this.fill--;
    this.model.loadEarlier();
  }

  /** The list index of the last turn's row: the spinner row ahead of the turns counts */
  private lastRow(): number {
    const view = this.model.view;
    return (view?.earlier !== undefined ? 1 : 0) + (view?.turns.length ?? 0) - 1;
  }

  /** A row's turn as the engine counts it, from the session's first turn: what its key, fold scope and actions carry */
  private at(i: number): number {
    return i + earlierTurns(this.model.view);
  }

  /** The thread's bottom padding: the to-do card floats over its foot, so the end stops above it */
  private footPad(): number {
    return Metrics.gap + (this.planShown() ? this.planHeight : 0);
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
    const choices = new ModelChoices(this.controls(), model.hiddenFor(model.view?.agent ?? ''),
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

  /**
   * Which stop the alert card speaks for: a retry that fails again leaves as many turns, but a new turn. The count is
   * the session's, so earlier turns landing in front of a windowed view bring no dismissed card back
   */
  private alertKey(turn: Turn): string {
    const view = this.model.view;
    return `${earlierTurns(view) + (view?.turns.length ?? 0)}:${turn.id ?? ''}:${turn.startedAt ?? ''}:${turn.endedAt ?? ''}`;
  }

  /** The to-do card shows (a plan or a goal): only then does its height push the thread's end up */
  private planShown(): boolean {
    const view = this.model.view;
    return view !== undefined && (dockPlan(view.turns, view.running) !== undefined || view.goal !== undefined);
  }

  private trackTop(): void {
    const moved = this.scroller.currentOffset().yOffset > 0;
    if (moved !== this.scrolled) {
      this.scrolled = moved;
    }
  }

  private receded(): boolean {
    return this.model.goo.open !== undefined;
  }

  build() {
    Column() {
      // The bar and its "…" menu (components/ChatBar.ets)
      ChatBar({ model: this.model, actions: this.actions })
        .blur(this.receded() ? Cmp.gooBlur : 0)

      if (this.model.view === undefined && this.model.unavailable !== undefined) {
        // The engine said why the session will not come: its sentence and a way to ask again (ChatScreen.swift)
        // A short headline carries the page; the engine's own sentence sits under it, small and in a narrow column
        Column({ space: Metrics.s1 }) {
          Text(t('mobile.session.unavailable'))
            .fontSize(Type.text1)
            .fontWeight(FontWeight.Medium)
            .fontColor($r('app.color.fg1'))
          Text(this.model.unavailable)
            .fontSize(Type.text3)
            .fontColor($r('app.color.fg3'))
            .textAlign(TextAlign.Center)
            .constraintSize({ maxWidth: Cmp.waitNoteW })
            .margin({ bottom: Metrics.s3 })
          CardButton({
            label: t('common.retry'),
            onTap: () => {
              const id = this.model.openId;
              if (id !== undefined) {
                this.model.open(id);
              }
            }
          })
        }
        .justifyContent(FlexAlign.Center)
        .width('100%')
        .layoutWeight(1)
      } else if (this.model.view === undefined) {
        LoadingProgress()
          .color($r('app.color.fg3'))
          .width(Metrics.barCtl)
          .height(Metrics.barCtl)
          .layoutWeight(1)
      } else {
        Stack({ alignContent: Alignment.Bottom }) {
          this.thread()
          // Messages leave under the bar through a fade, not a cut
          Column()
            .width('100%')
            .height(ListMetrics.topFadeTail)
            .linearGradient(topFade())
            .opacity(this.scrolled ? 1 : 0)
            .animation({ duration: ListMetrics.topFadeMs })
            .hitTestBehavior(HitTestMode.None)
            .position({ x: 0, y: 0 })
          // The to-do card floats over the thread's foot, headed by the session goal
          PlanDock({
            entries: dockPlan(this.model.view.turns, this.model.view.running), running: this.model.view.running,
            goal: this.model.view.goal,
            goalActions: this.model.view.goal !== undefined ? goalHeadActions(this.model.view.goal, this.model.view.goalActions) : [],
            onGoal: (action: string) => this.model.goal(action)
          })
            .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Cmp.dockGap })
            // A definite width under the cap: the card's own 100% resolves against this, not the whole Stack
            .width('100%')
            .constraintSize({ maxWidth: Metrics.contentW + Metrics.pad * 2 })
            .onAreaChange((_: Area, now: Area) => {
              this.planHeight = Number(now.height);
            })
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
            hidden: this.model.hiddenFor(this.model.view.agent),
            usage: this.model.view.usage,
            compactAt: this.model.compactAt,
            commands: this.model.view.commands ?? [],
            personas: this.model.personas,
            files: this.model.files,
            goo: this.model.goo,
            popup: this.popup,
            onSend: (text: string, staged: Attachment[]) => this.model.composerSend(text, staged),
            onStop: () => this.model.stop(),
            onCompact: () => this.model.compact(),
            onSetMode: (mode: string) => this.model.setMode(mode)
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
        // The component's own wrapper node covers the page too: closed, it must let touches through like its Stack
        .hitTestBehavior(this.model.goo.open !== undefined ? HitTestMode.Default : HitTestMode.None)
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }

  @Builder
  thread() {
    // A chat opens at its end, as the other apps do
    List({ scroller: this.scroller, space: Metrics.msgGap, initialIndex: Math.max(0, this.lastRow()) }) {
      // More of the session lies ahead of the first turn held: the row it loads under, its spinner out while it is asked for
      if (this.model.view?.earlier !== undefined) {
        ListItem() {
          LoadingProgress()
            .color($r('app.color.fg3'))
            .enableLoading(this.model.loadingEarlier)
            .width(Metrics.icon)
            .height(Metrics.icon)
        }
        .id('chat.earlier')
        .height(Metrics.lead)
      }
      // The first turn held may be an agent turn with only its trailing blocks, with no prompt ahead of it
      ForEach(this.model.view?.turns ?? [], (turn: Turn, i: number) => {
        ListItem() {
          if (turn.role === 'user') {
            UserBubble({ text: turn.text ?? '', turn: turn, thread: this.model.thread })
              .bindContextMenu(this.promptMenu(this.at(i), turn), ResponseType.LongPress)
          } else {
            AgentMessage({
              turn: turn,
              onPermission: (blockId: string, optionId: string) => this.model.permission(blockId, optionId),
              index: this.at(i),
              scope: this.model.view?.id ?? '',
              thread: this.model.thread,
              now: (): number => this.model.now(),
              actions: true,
              onFork: (turnIndex: number) => this.model.fork(turnIndex),
              onRetry: () => this.model.retry(),
              onBuild: (planId: string, optionId: string, configId: string, value: string) =>
                this.model.buildPlan(planId, optionId, configId, value)
            })
              .bindContextMenu(this.replyMenu(this.at(i)), ResponseType.LongPress)
          }
        }
        .constraintSize({ maxWidth: Metrics.contentW })
        .width('100%')
      }, (turn: Turn, i: number) => turnKey(turn, this.at(i)))
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
    .padding({
      left: Metrics.pad + Metrics.s1, right: Metrics.pad + Metrics.s1, top: Metrics.gap, bottom: this.footPad()
    })
    .scrollBar(BarState.Off)
    // A list shorter than the page takes no drag by default: with more of the session ahead, a pull at its top asks
    .edgeEffect(EdgeEffect.Spring, { alwaysEnabled: this.model.view?.earlier !== undefined })
    .onReachEnd(() => {
      this.atEnd = true;
    })
    .onScrollStart(() => {
      this.shortPull = this.short();
      this.pulled = false;
    })
    .onDidScroll((_: number, state: ScrollState) => {
      this.trackTop();
      this.readerScrolled(state);
    })
    // A chat opens at its end without a scroll of its own to report
    .onAppear(() => {
      setTimeout(() => this.trackTop(), 0);
    })
    .onScrollIndex((start: number, end: number) => {
      this.first = start;
      this.atEnd = end >= this.lastRow();
      // The first layout of a list just put on the page
      this.fillShort();
    })
    .onAreaChange((_: Area, now: Area) => {
      this.threadH = Number(now.height);
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
        this.dismissedAlert !== this.alertKey(this.alertTurn() as Turn)) {
        AlertCard({
          turn: this.alertTurn() as Turn,
          canCompact: (this.model.view.commands ?? []).some((c: SlashCommand) => c.name === 'compact'),
          onDismiss: () => {
            const turn = this.alertTurn();
            this.dismissedAlert = turn !== undefined ? this.alertKey(turn) : '';
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

  /**
   * A sent prompt's long press: rewrite it in the composer (while the session is ready and idle). `index` counts from
   * the session's first turn, here and in the reply's menu
   */
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
