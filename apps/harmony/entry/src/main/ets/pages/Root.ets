import { ListMetrics, Metrics } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { SessionList } from './SessionList';
import { ChatPage } from './ChatPage';
import { NewArgs, NewSession } from './NewSession';
import { AgentSettingsPage, ComingPage, DeveloperPage, SettingsPage, SettingsRoutes } from './Settings';
import { WelcomeFlow } from './Welcome';
import { ChatBarActions, Lead } from '../components/ChatBar';
import { ToastLayer } from '../components/Chrome';

// The app's frame (apps/ios RootView.swift, apps/android ui/Root.kt): the list, and the chat, a new session or settings pushed
// over it on a phone. A window at least splitMinW wide (tablets, foldables open, 2in1) keeps the list as a sidebar
// beside the detail column, which Navigation's Auto mode does at the same width; the sidebar hides from its bar and
// comes back from the detail root's leading button. The toast rides over every page, and the first run over all of it
// while the app follows no engine.

/** A page on the stack: its name says which, these say what it shows */
class Route {
  /** The detail column's first page beside the sidebar: it leads with the sidebar's button instead of back */
  root: boolean = false;
  /** An agent's settings, or a coming page's title key */
  id: string = '';
  agent: string | undefined = undefined;
  cwd: string | undefined = undefined;
}

@ComponentV2
export struct Root {
  @Require @Param model: AppModel;
  /** The first run over everything, until it finishes */
  @Param welcome: boolean = false;
  @Local wide: boolean = false;
  /** The sidebar hidden from its bar, the detail column taking the window */
  @Local hidden: boolean = false;
  @Local welcoming: boolean = false;
  private stack: NavPathStack = new NavPathStack();

  aboutToAppear(): void {
    this.welcoming = this.welcome;
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
      this.stack.pushPath({ name: 'chat', param: this.route(this.wide) });
    } else if (names[names.length - 1] === 'new') {
      this.stack.replacePath({ name: 'chat', param: this.topRoute() }, false);
    }
  }

  route(root: boolean): Route {
    const r = new Route();
    r.root = root;
    return r;
  }

  /** The top page's route, so a page replacing it keeps its place */
  topRoute(): Route {
    if (this.stack.size() === 0) {
      return this.route(this.wide);
    }
    const r = this.stack.getParamByIndex(this.stack.size() - 1) as Route | undefined;
    return this.route(r?.root ?? false);
  }

  /** A page picked from the list: pushed over it on a phone, the detail column's new root beside the sidebar */
  showRoot(name: string, route: Route): void {
    if (this.wide) {
      route.root = true;
      this.stack.clear(false);
      this.stack.pushPath({ name: name, param: route }, false);
    } else {
      this.stack.pushPath({ name: name, param: route });
    }
  }

  open(id: string): void {
    this.model.open(id);
    this.showRoot('chat', new Route());
  }

  newSession(agent: string | undefined, cwd: string | undefined): void {
    this.model.close();
    const r = new Route();
    r.agent = agent;
    r.cwd = cwd;
    this.showRoot('new', r);
  }

  push(name: string, id: string): void {
    const r = new Route();
    r.id = id;
    this.stack.pushPath({ name: name, param: r });
  }

  back(): void {
    const names = this.stack.getAllPathName();
    if (names.length > 0 && names[names.length - 1] === 'chat') {
      this.model.close();
    }
    if (this.wide && names.length <= 1) {
      // The detail column never stands empty: back at its root starts a new session
      this.stack.clear(false);
      this.stack.pushPath({ name: 'new', param: this.route(true) }, false);
      return;
    }
    this.stack.pop();
  }

  lead(route: Route): Lead {
    if (route.root && this.wide) {
      return this.hidden ? Lead.Sidebar : Lead.None;
    }
    return Lead.Back;
  }

  chatActions(route: Route): ChatBarActions {
    const a = new ChatBarActions();
    a.lead = this.lead(route);
    a.back = () => this.back();
    a.showSidebar = () => {
      this.hidden = false;
    };
    a.newSession = (agent: string, cwd: string | undefined) => this.newSession(agent.length > 0 ? agent : undefined, cwd);
    a.settings = () => this.push('settings', '');
    a.coming = (title: string) => this.push('coming', title);
    return a;
  }

  settingsRoutes(): SettingsRoutes {
    const r = new SettingsRoutes();
    r.agent = (id: string) => this.push('agent', id);
    r.developer = () => this.push('developer', '');
    r.coming = (title: string) => this.push('coming', title);
    return r;
  }

  @Builder
  page(name: string, param: Object) {
    if (name === 'chat') {
      NavDestination() {
        ChatPage({ model: this.model, showBack: !this.wide, actions: this.chatActions(param as Route), onBack: () => this.back() })
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
          args: this.newArgs(param as Route),
          lead: this.lead(param as Route),
          onBack: () => this.back(),
          onSidebar: () => {
            this.hidden = false;
          },
          onStart: (agent: string, cwd: string | undefined, text: string) => {
            this.model.startIn(agent, cwd, text);
            const names = this.stack.getAllPathName();
            if (names.length > 0 && names[names.length - 1] === 'new') {
              this.stack.replacePath({ name: 'chat', param: this.topRoute() }, false);
            }
          }
        })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.bg0'))
    } else if (name === 'settings') {
      NavDestination() {
        SettingsPage({ model: this.model, routes: this.settingsRoutes(), onBack: () => this.stack.pop() })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.form_page'))
    } else if (name === 'agent') {
      NavDestination() {
        AgentSettingsPage({ model: this.model, agentId: (param as Route).id, routes: this.settingsRoutes(), onBack: () => this.stack.pop() })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.form_page'))
    } else if (name === 'developer') {
      NavDestination() {
        DeveloperPage({ onBack: () => this.stack.pop() })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.form_page'))
    } else if (name === 'coming') {
      NavDestination() {
        ComingPage({ title: t((param as Route).id), onBack: () => this.stack.pop() })
      }
      .hideTitleBar(true)
      .backgroundColor($r('app.color.bg0'))
    }
  }

  newArgs(route: Route): NewArgs {
    const a = new NewArgs();
    a.agent = route.agent;
    a.cwd = route.cwd;
    return a;
  }

  build() {
    Stack({ alignContent: Alignment.Top }) {
      // A language pick rebuilds every page in the new table (apps/ios the root's `.id(locale)`)
      ForEach([this.model.language], (language: string) => {
        Navigation(this.stack) {
          SessionList({
            model: this.model,
            sidebar: this.wide,
            openId: this.wide ? this.model.openId : undefined,
            onOpen: (id: string) => this.open(id),
            onNew: (cwd: string | undefined) => this.newSession(undefined, cwd),
            onSettings: () => this.push('settings', ''),
            onHide: () => {
              this.hidden = true;
            }
          })
        }
        .mode(NavigationMode.Auto)
        .navBarWidth(Metrics.sidebarW)
        .minContentWidth(Metrics.splitMinW - Metrics.sidebarW)
        .hideNavBar(this.wide && this.hidden)
        .hideTitleBar(true)
        .hideToolBar(true)
        .hideBackButton(true)
        .navDestination(this.page)
        .backgroundColor($r('app.color.bg0'))
        .onNavigationModeChange((mode: NavigationMode) => {
          this.wide = mode === NavigationMode.Split;
          // The detail column never stands empty beside the sidebar: its first page is a new session
          if (this.wide && this.stack.size() === 0) {
            this.stack.pushPath({ name: 'new', param: this.route(true) }, false);
          }
        })
      }, (language: string) => language)
      ToastLayer({ model: this.model })
      if (this.welcoming) {
        WelcomeFlow({
          model: this.model, onDone: () => {
            this.welcoming = false;
          }
        })
          .transition(TransitionEffect.OPACITY.animation({ duration: this.model.motion ? ListMetrics.motionMs : 0 }))
      }
    }
    .width('100%')
    .height('100%')
  }
}
