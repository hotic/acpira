import { Metrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { ChatEntries, SessionMenu } from './SessionMenu';
import { CapsuleItem, GlassButton, GlassCapsule } from './Chrome';
import { menuOptions } from './Menu';

// The chat's bar (apps/ios Chat/ChatScreen.swift's toolbar, apps/android ui/ChatScreen.kt TopAppBar): the leading
// button (back on a stacked page, the sidebar at a wide window's detail root while it is hidden, nothing beside an open
// one), the title, then a glass capsule with a new session and the session's "…" menu (components/SessionMenu.ets).

/** What leads a page's bar */
export enum Lead {
  Back,
  Sidebar,
  None
}

/** Where the bar's entries lead; the root page (pages/Index.ets) fills them in */
export class ChatBarActions {
  lead: Lead = Lead.Back;
  back: () => void = () => {};
  showSidebar: () => void = () => {};
  /** A new session, with the chat's agent and folder */
  newSession: (agent: string, cwd: string | undefined) => void = () => {};
  settings: () => void = () => {};
  /** A page another part of the app builds, by its title key (account, changes, subagents, terminal) */
  coming: (title: string) => void = () => {};
}

/** A bar's leading button: back, show the sidebar, or nothing */
@ComponentV2
export struct LeadButton {
  @Param lead: Lead = Lead.Back;
  @Param tag: string = 'bar.back';
  @Event onBack: () => void = () => {};
  @Event onSidebar: () => void = () => {};

  build() {
    if (this.lead === Lead.Back) {
      GlassButton({ icon: 'arrowLeft', tag: this.tag, onTap: () => this.onBack() })
    } else if (this.lead === Lead.Sidebar) {
      GlassButton({ icon: 'panelLeft', tag: 'split.open', onTap: () => this.onSidebar() })
    }
  }
}

@ComponentV2
export struct ChatBar {
  @Require @Param model: AppModel;
  @Param actions: ChatBarActions = new ChatBarActions();
  @Local menu: boolean = false;

  entries(): ChatEntries {
    const e = new ChatEntries();
    const actions = this.actions;
    e.account = () => actions.coming('common.account');
    e.changes = (this.model.view?.cwd ?? '').length > 0 ? () => actions.coming('panel.changes') : undefined;
    e.terminal = (this.model.view?.cwd ?? '').length > 0 ? () => actions.coming('panel.terminal') : undefined;
    e.settings = () => actions.settings();
    e.delete = true;
    return e;
  }

  @Builder
  menuCard() {
    SessionMenu({
      model: this.model,
      sessionId: this.model.view?.id ?? this.model.openId ?? '',
      chat: this.entries(),
      onClose: () => {
        this.menu = false;
      },
      // An archived chat leaves the list; going back shows where it went
      onArchived: () => this.actions.back()
    })
  }

  build() {
    Row({ space: Metrics.s2 }) {
      LeadButton({ lead: this.actions.lead, tag: 'chat.back', onBack: () => this.actions.back(), onSidebar: () => this.actions.showSidebar() })
      Text(this.model.view?.title ?? '')
        .id('chat.title')
        .fontSize(Type.text1)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg_strong'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .textAlign(TextAlign.Center)
        .layoutWeight(1)
      GlassCapsule() {
        CapsuleItem({
          icon: 'squarePen', tag: 'chat.new', onTap: () => {
            const view = this.model.view;
            const cwd = view === undefined || this.model.isChat(view.cwd) ? undefined : view.cwd;
            this.actions.newSession(view?.agent ?? '', cwd);
          }
        })
        CapsuleItem({ icon: 'ellipsis', tag: 'chat.more', onTap: () => { this.menu = true; } })
          .bindMenu(this.menu, this.menuCard(), menuOptions(() => { this.menu = false; }))
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .height(Metrics.barCtl + Metrics.gap)
    .width('100%')
  }
}
