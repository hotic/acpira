import { pasteboard } from '@kit.BasicServicesKit';
import { common } from '@kit.AbilityKit';
import { t } from '../store/L10n';

// The transcript's system actions: copy to the pasteboard (with the "Copied" toast the other apps show) and open a link
// in the browser.

export function copyText(ui: UIContext, text: string): void {
  try {
    pasteboard.getSystemPasteboard().setData(pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, text)).then(() => {
      ui.getPromptAction().showToast({ message: t('code.copied') });
    }).catch(() => {});
  } catch (e) {
    // No pasteboard (the Previewer): nothing to confirm
  }
}

/** A Markdown link: web addresses open in the browser; anything else (a file path) is not followed here */
export function openLink(ui: UIContext, url: string): void {
  if (!/^(https?|mailto):/i.test(url)) {
    return;
  }
  try {
    (ui.getHostContext() as common.UIAbilityContext).openLink(url).catch(() => {});
  } catch (e) {
    // No ability (the Previewer)
  }
}
