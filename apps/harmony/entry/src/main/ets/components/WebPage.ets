import { webview } from '@kit.ArkWeb';
import { Metrics, Type } from '../design/Tokens';
import { GlassButton } from './Chrome';

// The in-app browser (apps/ios Web/WebPage.swift, apps/android ui/Links.kt): a reply's web link opens over the chat,
// which stays one close away. HarmonyOS has no system browser sheet to borrow, so the page is the app's own ArkWeb.
// Root pushes it as a page when `webPage.open` turns on.

/** The page the in-app browser shows, and whether it is up */
@ObservedV2
export class WebPageState {
  @Trace url: string = '';
  @Trace open: boolean = false;

  show(url: string): void {
    this.url = url;
    this.open = true;
  }
}

export const webPage: WebPageState = new WebPageState();

/** A page a browser shows: the in-app browser takes http and https only */
export function isWebPage(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

/** The host a bar names for a page */
function hostOf(url: string): string {
  const m = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]+)/i.exec(url);
  return m !== null ? m[1] : url;
}

@ComponentV2
export struct WebPageView {
  @Param url: string = '';
  @Event onClose: () => void = () => {};
  /** The system browser, for the page now shown (it may have moved on from `url`) */
  @Event onOutside: (url: string) => void = () => {};
  @Local current: string = '';
  @Local progress: number = 0;
  private controller: webview.WebviewController = new webview.WebviewController();

  aboutToAppear(): void {
    this.current = this.url;
  }

  build() {
    Column() {
      Row({ space: Metrics.s2 }) {
        GlassButton({ icon: 'x', tag: 'web.close', onTap: () => this.onClose() })
        Text(hostOf(this.current))
          .id('web.host')
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .textAlign(TextAlign.Center)
          .layoutWeight(1)
        GlassButton({ icon: 'compass', tag: 'web.outside', onTap: () => this.onOutside(this.current) })
      }
      .padding({ left: Metrics.pad, right: Metrics.pad })
      .height(Metrics.barCtl + Metrics.gap)
      .width('100%')
      // The load's progress, gone once the page is in
      Progress({ value: this.progress, total: 100, type: ProgressType.Linear })
        .color($r('app.color.command'))
        .height(Metrics.hairline * 2)
        .width('100%')
        .opacity(this.progress > 0 && this.progress < 100 ? 1 : 0)
      Web({ src: this.url, controller: this.controller })
        .id('web.page')
        // The app's colour mode (AppModel sets it on the application): a page with a dark style shows it
        .darkMode(WebDarkMode.Auto)
        .domStorageAccess(true)
        .onProgressChange((e) => {
          this.progress = e?.newProgress ?? 0;
        })
        .onPageBegin((e) => {
          this.current = e?.url ?? this.current;
        })
        .layoutWeight(1)
        .width('100%')
    }
    .width('100%')
    .height('100%')
    .backgroundColor($r('app.color.bg0'))
  }
}
