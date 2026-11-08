import { Metrics } from '../design/Tokens';
import { i18n } from '@kit.LocalizationKit';
import { AppModel, appModel } from '../store/AppModel';
import { initL10n } from '../store/L10n';
import { SessionList } from './SessionList';
import { ChatPage } from './ChatPage';
import { NewSession } from './NewSession';
import { DEV_ENGINE, DEV_PROMPT } from '../generated/Assets.generated';

// The root (apps/ios RootView.swift, apps/android ui/Root.kt): the list, and the chat pushed over it on a phone. A
// window at least splitMinW wide (tablets, foldables open, 2in1) keeps the list as a sidebar beside the chat, which
// Navigation's Auto mode does at the same width.

@Entry
@ComponentV2
struct Index {
  @Local model: AppModel = appModel;
  @Local wide: boolean = false;
  private stack: NavPathStack = new NavPathStack();

  aboutToAppear(): void {
    // Here rather than in EntryAbility: the Previewer renders the page without running the ability
    initL10n(i18n.System.getSystemLanguage());
    // `engine`: `host:port:token` of the development engine (apps/ios/scripts/engine.sh); without one the demo shows
    const launch = AppStorage.get<Record<string, Object>>('launch') ?? {};
    // (or the one a development build compiled in, scripts/build.sh DEV_ENGINE)
    const engine = (launch['engine'] as string | undefined) ?? (DEV_ENGINE.length > 0 ? DEV_ENGINE : undefined);
    const parts = engine?.split(':') ?? [];
    if (parts.length === 3) {
      this.model.startOnConnect = DEV_PROMPT.length > 0 ? DEV_PROMPT : undefined;
      this.model.connect(`${parts[0]}:${parts[1]}`, parts[2], i18n.System.getSystemLanguage());
    } else {
      this.model.demo();
    }
  }

  /**
   * A session the model opened by itself (a first prompt's) comes up as the chat: over the list on a phone, in place of
   * the new session page the detail column of a wide window starts on
   */
  @Monitor('model.openId')
  onOpened(): void {
    if (this.model.openId === undefined) {
      return;
    }
    const names = this.stack.getAllPathName();
    if (names.length === 0) {
      this.stack.pushPath({ name: 'chat' });
    } else if (names[names.length - 1] === 'new') {
      this.stack.replacePath({ name: 'chat' }, false);
    }
  }

  open(id: string): void {
    this.model.open(id);
    if (this.stack.size() > 0) {
      this.stack.replacePath({ name: 'chat' }, false);
    } else {
      this.stack.pushPath({ name: 'chat' });
    }
  }

  back(): void {
    this.model.close();
    this.stack.pop();
  }

  @Builder
  page(name: string) {
    if (name === 'chat') {
      NavDestination() {
        ChatPage({ model: this.model, showBack: !this.wide, onBack: () => this.back() })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.bg0'))
      .onBackPressed(() => {
        this.back();
        return true;
      })
    } else if (name === 'new') {
      NavDestination() {
        NewSession({
          model: this.model,
          showBack: !this.wide,
          onBack: () => this.stack.pop(),
          onStart: (text: string) => {
            this.model.start(text);
            this.stack.replacePath({ name: 'chat' }, false);
          }
        })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.bg0'))
    }
  }

  build() {
    Navigation(this.stack) {
      SessionList({
        model: this.model,
        openId: this.wide ? this.model.openId : undefined,
        onOpen: (id: string) => this.open(id),
        onNew: () => {
          this.model.close();
          if (this.stack.size() > 0) {
            this.stack.replacePath({ name: 'new' }, false);
          } else {
            this.stack.pushPath({ name: 'new' });
          }
        }
      })
    }
    .mode(NavigationMode.Auto)
    .navBarWidth(Metrics.sidebarW)
    .minContentWidth(Metrics.splitMinW - Metrics.sidebarW)
    .hideTitleBar(true)
    .hideToolBar(true)
    .hideBackButton(true)
    .navDestination(this.page)
    .backgroundColor($r('app.color.bg0'))
    .onNavigationModeChange((mode: NavigationMode) => {
      this.wide = mode === NavigationMode.Split;
      // The detail column never stands empty beside the sidebar: its first page is a new session
      if (this.wide && this.stack.size() === 0) {
        this.stack.pushPath({ name: 'new' }, false);
      }
    })
  }
}
