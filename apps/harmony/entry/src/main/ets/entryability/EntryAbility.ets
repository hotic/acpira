import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { KeyboardAvoidMode, window } from '@kit.ArkUI';
import { emitter } from '@kit.BasicServicesKit';
import { PAIR_LINK_EVENT } from '../store/PairingModel';

/** An `acpira://pair/…` link from outside (a code scanned in the camera app, a tapped link) or the `pair` launch parameter */
function pairLinkOf(want: Want): string | undefined {
  const uri = want.uri ?? '';
  if (uri.startsWith('acpira://pair/')) {
    return uri;
  }
  const param = want.parameters?.['pair'];
  return typeof param === 'string' ? param : undefined;
}

/** The one ability: a full-screen window drawing every page (pages/Index.ets) */
export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    const launch: Record<string, Object> = want.parameters ?? {};
    const link = pairLinkOf(want);
    if (link !== undefined) {
      launch['pair'] = link;
    }
    AppStorage.setOrCreate('launch', launch);
  }

  /** A pairing link while the app runs: the page opens the add sheet on it */
  onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    const link = pairLinkOf(want);
    if (link !== undefined) {
      emitter.emit(PAIR_LINK_EVENT, { data: { 'link': link } });
    }
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
