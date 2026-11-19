import { pasteboard } from '@kit.BasicServicesKit';
import { common } from '@kit.AbilityKit';
import { t } from '../store/L10n';
import { isWebPage, webPage } from './WebPage';

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

/** A Markdown link: a web page in the in-app browser, mail with the system; anything else (a file path) is not followed */
export function openLink(ui: UIContext, url: string): void {
  if (isWebPage(url)) {
    webPage.show(url);
  } else if (/^mailto:/i.test(url)) {
    openOutside(ui, url);
  }
}

/** The system browser, or the app that claims the link */
export function openOutside(ui: UIContext, url: string): void {
  try {
    (ui.getHostContext() as common.UIAbilityContext).openLink(url).catch(() => {});
  } catch (e) {
    // No ability (the Previewer)
  }
}

/**
 * The web link under the last press on a reply's text (apps/ios TextSelectionScope.link). A long press on text opens
 * the system's selection menu, never the reply's, so that menu leads with the link's actions (Prose InlineText)
 */
export class LinkPress {
  url: string = '';
}

export const linkPress: LinkPress = new LinkPress();
