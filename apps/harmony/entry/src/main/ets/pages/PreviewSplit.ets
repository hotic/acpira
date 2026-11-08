import { i18n } from '@kit.LocalizationKit';
import { AppModel } from '../store/AppModel';
import { initL10n } from '../store/L10n';
import { Root } from './Root';

// The app's frame on the demo without the first run over it, which pages/Index shows in a Previewer (it keeps no
// prefs): a wide size gives the sidebar split. Not reached from the app's own pages.

@Entry
@ComponentV2
struct PreviewSplit {
  @Local model: AppModel = new AppModel();

  aboutToAppear(): void {
    const language = i18n.System.getSystemLanguage();
    initL10n(language);
    this.model.attach(undefined, language);
    this.model.demo();
  }

  build() {
    // An entry's root must be a container
    Stack() {
      Root({ model: this.model })
    }
  }
}
