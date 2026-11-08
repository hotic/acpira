import { i18n } from '@kit.LocalizationKit';
import { emitter } from '@kit.BasicServicesKit';
import { AppModel, appModel } from '../store/AppModel';
import { PAIR_LINK_EVENT } from '../store/PairingModel';
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
    // build compiled in (scripts/build.sh DEV_ENGINE), or the one the app was set to follow; then the paired computers
    // (when the build carries the remote access library); without any the demo shows
    // A link opened later (EntryAbility `onNewWant`) goes to the add sheet the same way; the first run's pairing step
    // takes it while that is up. Listened to whatever is followed now: forgetting the engine hands over to pairing
    emitter.on(PAIR_LINK_EVENT, (event: emitter.EventData) => {
      const later = event.data?.['link'] as string | undefined;
      if (later !== undefined) {
        this.model.pairing?.open(later);
      }
    });
    const launch = AppStorage.get<Record<string, Object>>('launch') ?? {};
    const asked = (launch['engine'] as string | undefined) ?? (DEV_ENGINE.length > 0 ? DEV_ENGINE : undefined);
    const engine = asked ?? this.model.storedEngine();
    const parts = engine?.split(':') ?? [];
    if (parts.length === 3) {
      this.model.startOnConnect = DEV_PROMPT.length > 0 ? DEV_PROMPT : undefined;
      this.model.follow(`${parts[0]}:${parts[1]}`, parts[2], false);
      return;
    }
    if (!this.model.startPairing()) {
      this.model.demo();
    }
    // `pair`: an `acpira://pair/…` link to confirm in the add sheet (opened from outside, or `aa start … --ps pair <link>`)
    const link = launch['pair'] as string | undefined;
    const opened = link !== undefined && this.model.pairing !== undefined && this.model.pairing.open(link);
    // The first run, while nothing is followed and it never finished
    this.welcome = !opened && !this.model.paired && welcomeDue();
  }

  aboutToDisappear(): void {
    emitter.off(PAIR_LINK_EVENT);
  }

  onPageShow(): void {
    this.model.setAway(false);
    this.model.pairing?.enterForeground();
  }

  onPageHide(): void {
    // The chat on screen is no longer watched: a turn ending meanwhile keeps its unread mark (docs/dev/ios.md)
    this.model.setAway(true);
    this.model.pairing?.enterBackground();
  }

  build() {
    // An entry's root must be a container
    Stack() {
      Root({ model: this.model, welcome: this.welcome })
    }
  }
}
