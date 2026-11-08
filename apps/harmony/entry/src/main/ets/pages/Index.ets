import { i18n } from '@kit.LocalizationKit';
import { AppModel, appModel } from '../store/AppModel';
import { initL10n } from '../store/L10n';
import { Root } from './Root';
import { welcomeDue } from './Welcome';
import { DEV_ENGINE, DEV_PROMPT } from '../generated/Assets.generated';

// The app's page: picks the engine to follow, then hands the frame (pages/Root.ets) the model.

@Entry
@ComponentV2
struct Index {
  @Local model: AppModel = appModel;
  @Local welcome: boolean = false;

  aboutToAppear(): void {
    // Here rather than in EntryAbility: the Previewer renders the page without running the ability
    const language = i18n.System.getSystemLanguage();
    initL10n(language);
    this.model.attach(this.getUIContext().getHostContext(), language);
    // `engine`: `host:port:token` of the development engine (apps/ios/scripts/engine.sh), or the one a development
    // build compiled in (scripts/build.sh DEV_ENGINE), or the one the app was set to follow; without one the demo shows
    const launch = AppStorage.get<Record<string, Object>>('launch') ?? {};
    const asked = (launch['engine'] as string | undefined) ?? (DEV_ENGINE.length > 0 ? DEV_ENGINE : undefined);
    const engine = asked ?? this.model.storedEngine();
    const parts = engine?.split(':') ?? [];
    if (parts.length === 3) {
      this.model.startOnConnect = DEV_PROMPT.length > 0 ? DEV_PROMPT : undefined;
      this.model.follow(`${parts[0]}:${parts[1]}`, parts[2], false);
    } else {
      this.model.demo();
      // The first run, while no engine is followed and it never finished
      this.welcome = welcomeDue();
    }
  }

  build() {
    // An entry's root must be a container
    Stack() {
      Root({ model: this.model, welcome: this.welcome })
    }
  }
}
