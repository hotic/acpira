import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { KeyboardAvoidMode, window } from '@kit.ArkUI';

/** The one ability: a full-screen window drawing every page (pages/Index.ets) */
export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    AppStorage.setOrCreate('launch', want.parameters ?? {});
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index', (err) => {
      if (err.code) {
        console.error(`acpira: loadContent failed ${err.code}`);
        return;
      }
      // The keyboard shortens the page (the thread above the composer) instead of lifting it, bar and all, off screen
      windowStage.getMainWindowSync().getUIContext().setKeyboardAvoidMode(KeyboardAvoidMode.RESIZE);
    });
  }
}
