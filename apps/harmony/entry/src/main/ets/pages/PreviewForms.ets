import { i18n } from '@kit.LocalizationKit';
import { initL10n } from '../store/L10n';
import { Scene } from './PreviewShell';

// More Previewer frames (pages/PreviewShell.ets): settings, a new session with its agent ring open, the chat's bar and
// the first run's later steps. Not reached from the app's own pages.

@Entry
@ComponentV2
struct PreviewForms {
  aboutToAppear(): void {
    initL10n(i18n.System.getSystemLanguage());
  }

  build() {
    Row() {
      ForEach(['settings', 'agent', 'new', 'chat', 'intro', 'engine', 'notify'], (scene: string) => {
        Scene({ scene: scene })
      }, (scene: string) => scene)
    }
    .height('100%')
    .alignItems(VerticalAlign.Top)
  }
}
