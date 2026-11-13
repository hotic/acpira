import { i18n } from '@kit.LocalizationKit';
import { AppModel } from '../store/AppModel';
import { initL10n } from '../store/L10n';
import { Root } from './Root';

// The app's frame with a demo chat open (pages/PreviewSplit.ets shows the list): over the list in a phone-wide
// window, beside the sidebar in a wide one, where its row is the lit one. Not reached from the app's own pages.

/** The frame is up before the chat opens: Root follows the model's open chat from its first change on */
const OPEN_AFTER_MS: number = 300;

@Entry
@ComponentV2
struct PreviewSplitChat {
  @Local model: AppModel = new AppModel();

  aboutToAppear(): void {
    const language = i18n.System.getSystemLanguage();
    initL10n(language);
    this.model.attach(undefined, language);
    this.model.demo();
    setTimeout(() => this.model.open(this.model.summaries[3]?.id ?? ''), OPEN_AFTER_MS);
  }

  build() {
    // An entry's root must be a container
    Stack() {
      Root({ model: this.model })
    }
  }
}
