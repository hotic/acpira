import { i18n } from '@kit.LocalizationKit';
import { AppModel } from '../store/AppModel';
import { fixtureView } from '../store/Demo';
import { initL10n } from '../store/L10n';
import { ChatPage } from './ChatPage';

// The Previewer cannot tap, so chat fixtures open here directly (scripts/preview.sh): the showcase above, a running
// turn waiting on a permission below. Not reached from the app's own pages.

function modelOn(fixture: string): AppModel {
  const model = new AppModel();
  model.demo();
  model.open(fixture);
  model.view = fixtureView(fixture);
  return model;
}

@Entry
@ComponentV2
struct Preview {
  @Local top: AppModel = new AppModel();
  @Local bottom: AppModel = new AppModel();

  aboutToAppear(): void {
    initL10n(i18n.System.getSystemLanguage());
    this.top = modelOn('showcase');
    this.bottom = modelOn('running');
  }

  build() {
    Column() {
      ChatPage({ model: this.top, showBack: true }).height('35%')
      Divider().color($r('app.color.line_strong'))
      ChatPage({ model: this.bottom, showBack: true }).layoutWeight(1)
    }
    .height('100%')
    .width('100%')
  }
}
