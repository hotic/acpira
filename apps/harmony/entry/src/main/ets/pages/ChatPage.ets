import { Turn } from '../wire/Wire';
import { Metrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { Lucide } from '../components/Glyphs';
import { AgentMessage, UserBubble } from '../components/TurnViews';
import { Composer } from '../components/Composer';
import { ChatBar, ChatBarActions } from '../components/ChatBar';

// The chat (apps/ios Chat/ChatView.swift): the bar with back and the title, the thread in the reading column, the
// composer under it. The thread follows new output while it is at its end.

@ComponentV2
export struct ChatPage {
  @Require @Param model: AppModel;
  /** False at the detail column's root beside an open sidebar: nothing to go back to */
  @Param showBack: boolean = true;
  @Event onBack: () => void = () => {};
  /** Where the bar's entries lead (list agent) */
  @Param actions: ChatBarActions = new ChatBarActions();
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

  build() {
    Column() {
      // The bar and its "…" menu (components/ChatBar.ets)
      ChatBar({ model: this.model, actions: this.actions })

      if (this.model.view === undefined) {
        LoadingProgress()
          .color($r('app.color.fg3'))
          .width(Metrics.barCtl)
          .height(Metrics.barCtl)
          .layoutWeight(1)
      } else {
        List({ scroller: this.scroller, space: Metrics.msgGap }) {
          ForEach(this.model.view.turns, (turn: Turn, i: number) => {
            ListItem() {
              if (turn.role === 'user') {
                UserBubble({ text: turn.text ?? '' })
              } else {
                AgentMessage({
                  turn: turn,
                  running: (this.model.view?.running ?? false) && i === (this.model.view?.turns.length ?? 0) - 1,
                  onPermission: (blockId: string, optionId: string) => this.model.permission(blockId, optionId)
                })
              }
            }
            .constraintSize({ maxWidth: Metrics.contentW })
            .width('100%')
          }, (turn: Turn, i: number) => this.turnKey(turn, i))
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
        .layoutWeight(1)
      }

      Row() {
        Composer({
          placeholder: t('composer.placeholder'),
          running: this.model.view?.running ?? false,
          onSend: (text: string) => this.model.send(text),
          onStop: () => this.model.stop()
        })
      }
      .justifyContent(FlexAlign.Center)
      .padding({ left: Metrics.gap, right: Metrics.gap, top: Metrics.s2, bottom: Metrics.s2 })
      .width('100%')
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }
}
