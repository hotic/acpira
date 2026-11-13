import { i18n } from '@kit.LocalizationKit';
import { AppModel, Connection } from '../store/AppModel';
import { initL10n, t } from '../store/L10n';
import { fixtureView } from '../store/Demo';
import { SessionList } from './SessionList';
import { SettingsPage, AgentSettingsPage } from './Settings';
import { NewSession } from './NewSession';
import { ChatPage } from './ChatPage';
import { WelcomeFlow } from './Welcome';
import { SessionMenu } from '../components/SessionMenu';
import { MenuCard } from '../components/Menu';
import { ToastLayer } from '../components/Chrome';
import { ChatBarActions } from '../components/ChatBar';
import { Metrics } from '../design/Tokens';
import { PREVIEW_FRAME } from '../generated/Assets.generated';

// The Previewer cannot tap, so the shell around the chat shows here as phone-wide frames side by side
// (scripts/preview.sh <out> 3780 1170 dark pages/PreviewShell 240): every frame its own demo model in one state.
// Not reached from the app's own pages.

/** A frame's width in vp: a phone's (1080 px at 480 dpi), or the build's PREVIEW_FRAME (326 for a small square screen) */
const PHONE: number = PREVIEW_FRAME > 0 ? PREVIEW_FRAME : 360;

function demoModel(): AppModel {
  const model = new AppModel();
  model.attach(undefined, i18n.System.getSystemLanguage());
  model.demo();
  return model;
}

/** One frame: a page in a state, named by `scene` */
@ComponentV2
export struct Scene {
  @Param scene: string = 'list';
  @Local model: AppModel = new AppModel();

  aboutToAppear(): void {
    const model = demoModel();
    if (this.scene === 'list') {
      model.lists.toast(t('session.archived', new Map<string, string>([['title', model.summaries[2]?.title ?? '']])), t('common.undo'), () => {});
    } else if (this.scene === 'project') {
      model.lists.view = 'project';
    } else if (this.scene === 'closed') {
      model.connection = Connection.Closed;
      model.lists.view = 'agent';
    } else if (this.scene === 'chat') {
      const id = model.summaries[1]?.id ?? '';
      model.open(id);
      model.view = fixtureView('showcase');
    }
    this.model = model;
  }

  firstId(): string {
    return this.model.summaries[1]?.id ?? '';
  }

  build() {
    Stack({ alignContent: Alignment.Top }) {
      if (this.scene === 'list' || this.scene === 'project' || this.scene === 'closed') {
        SessionList({ model: this.model })
      } else if (this.scene === 'search') {
        SessionList({ model: this.model, initialQuery: '解析' })
      } else if (this.scene === 'menu' || this.scene === 'copy') {
        SessionList({ model: this.model })
        Column() {
          MenuCard() {
            SessionMenu({ model: this.model, sessionId: this.firstId(), start: this.scene === 'copy' ? 'copy' : '' })
          }
        }
        .width('100%')
        .height('100%')
        .padding({ top: Metrics.msgGap * 4, left: Metrics.pad * 3 })
        .backgroundColor($r('app.color.goo_dim'))
      } else if (this.scene === 'settings') {
        SettingsPage({ model: this.model })
      } else if (this.scene === 'agent') {
        AgentSettingsPage({ model: this.model, agentId: 'claude' })
      } else if (this.scene === 'new') {
        NewSession({ model: this.model, ringOnLaunch: true })
      } else if (this.scene === 'chat') {
        ChatPage({ model: this.model, showBack: true, actions: new ChatBarActions() })
      } else if (this.scene === 'engine' || this.scene === 'notify' || this.scene === 'intro') {
        WelcomeFlow({ model: this.model, start: this.scene })
      }
      ToastLayer({ model: this.model })
    }
    .width(PHONE)
    .height('100%')
    .clip(true)
    .backgroundColor(this.scene === 'settings' || this.scene === 'agent' ? $r('app.color.form_page') : $r('app.color.bg0'))
  }
}

@Entry
@ComponentV2
struct PreviewShell {
  aboutToAppear(): void {
    initL10n(i18n.System.getSystemLanguage());
  }

  build() {
    Row() {
      ForEach(['list', 'project', 'search', 'menu', 'copy', 'closed'], (scene: string) => {
        Scene({ scene: scene })
      }, (scene: string) => scene)
    }
    .height('100%')
    .alignItems(VerticalAlign.Top)
  }
}
